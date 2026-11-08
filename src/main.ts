import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { open as openBrowser } from "@tauri-apps/plugin-shell";

import { EditorState, Compartment } from "@codemirror/state";
import { EditorView, lineNumbers, highlightActiveLine, keymap } from "@codemirror/view";
import { defaultKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, foldGutter } from "@codemirror/language";
import { closeBrackets } from "@codemirror/autocomplete";
import { oneDark } from "@codemirror/theme-one-dark";

import { rust } from "@codemirror/lang-rust";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { cpp } from "@codemirror/lang-cpp";

interface DocumentMetadata {
  id: string;
  title: string;
  file_path: string | null;
  gdrive_file_id: string | null;
  encoding: string;
  total_lines: number;
  total_chars: number;
  total_bytes: number;
  is_modified: boolean;
}

interface TabContentResponse {
  metadata: DocumentMetadata;
  content: string;
}

interface GDriveFile {
  id: string;
  name: string;
  mimeType: string;
}

interface CustomFontData {
  font_family: string;
  data_url: string;
}

let activeTabId: string = "";
let activeMeta: DocumentMetadata | null = null;
let isInternalChange = false;
let debounceTimer: number | undefined;

const languageCompartment = new Compartment();
let editorView: EditorView;

// 🛠️ [KHU VỰC DỄ CHỈNH SỬA 4]: Bảng mã màu mặc định của chế độ Dark
const DEFAULT_DARK_COLORS: Record<string, string> = {
  "--bg-primary": "#1e1e2e",
  "--bg-secondary": "#181825",
  "--bg-surface": "#313244",
  "--text-main": "#cdd6f4",
  "--accent": "#89b4fa",
  "--border": "#45475a",
};

let currentCustomColors: Record<string, string> = { ...DEFAULT_DARK_COLORS };

// 🛠️ [KHU VỰC DỄ CHỈNH SỬA 5]: Nhận diện đuôi file để tô màu code
function detectLanguageExtension(filename: string) {
  const ext = filename.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "rs": return rust();
    case "js": case "ts": case "jsx": case "tsx": return javascript({ typescript: true });
    case "py": return python();
    case "html": case "htm": return html();
    case "css": return css();
    case "json": return json();
    case "md": case "txt": return markdown();
    case "c": case "cpp": case "h": case "hpp": return cpp();
    default: return markdown();
  }
}

function initEditor() {
  const host = document.getElementById("editor-host")!;
  const state = EditorState.create({
    doc: "",
    extensions: [
      lineNumbers(),
      highlightActiveLine(),
      foldGutter(),
      bracketMatching(),
      closeBrackets(),
      EditorView.lineWrapping,
      oneDark,
      languageCompartment.of(markdown()),
      keymap.of([...defaultKeymap, indentWithTab]),
      EditorView.updateListener.of((update) => {
        if (update.selectionSet) {
          updateCursorPosition();
        }
        if (update.docChanged && !isInternalChange && activeTabId) {
          clearTimeout(debounceTimer);
          debounceTimer = window.setTimeout(async () => {
            const text = editorView.state.doc.toString();
            const meta = await invoke<DocumentMetadata>("update_buffer", {
              tabId: activeTabId,
              content: text,
            });
            updateStatusBar(meta);
            await refreshTabBar();
          }, 250);
        }
      }),
    ],
  });

  editorView = new EditorView({ state, parent: host });
}

function setEditorText(content: string, filename: string) {
  isInternalChange = true;
  editorView.dispatch({
    changes: { from: 0, to: editorView.state.doc.length, insert: content },
    effects: languageCompartment.reconfigure(detectLanguageExtension(filename)),
  });
  isInternalChange = false;
}

function updateCursorPosition() {
  const pos = editorView.state.selection.main.head;
  const line = editorView.state.doc.lineAt(pos);
  const col = pos - line.from + 1;
  document.getElementById("stat-cursor")!.textContent = `Dòng ${line.number}, Cột ${col}`;
}

function updateStatusBar(meta: DocumentMetadata) {
  activeMeta = meta;
  document.getElementById("stat-lines")!.textContent = `${meta.total_lines} dòng | ${meta.total_bytes} B`;
  document.getElementById("stat-encoding")!.textContent = meta.encoding;
  document.getElementById("stat-sync")!.textContent = meta.gdrive_file_id ? "☁️ Google Drive" : "📱 Local";
}

async function refreshTabBar() {
  const tabs = await invoke<DocumentMetadata[]>("get_all_tabs");
  const tabBar = document.getElementById("tab-bar")!;
  tabBar.innerHTML = "";

  tabs.forEach((tab) => {
    const item = document.createElement("div");
    item.className = `tab-item ${tab.id === activeTabId ? "active" : ""}`;

    const label = document.createElement("span");
    label.textContent = `${tab.is_modified ? "● " : ""}${tab.title}`;
    item.appendChild(label);

    const closeBtn = document.createElement("span");
    closeBtn.className = "tab-close";
    closeBtn.textContent = "✕";
    closeBtn.onclick = async (e) => {
      e.stopPropagation();
      const nextId = await invoke<string | null>("close_tab", { tabId: tab.id });
      if (nextId) {
        await switchToTab(nextId);
      }
    };

    item.onclick = () => switchToTab(tab.id);
    item.appendChild(closeBtn);
    tabBar.appendChild(item);
  });
}

async function switchToTab(tabId: string) {
  const res = await invoke<TabContentResponse>("switch_tab", { tabId });
  activeTabId = res.metadata.id;
  setEditorText(res.content, res.metadata.title);
  updateStatusBar(res.metadata);
  await refreshTabBar();
}

function rgbToHex(r: number, g: number, b: number): string {
  const toHex = (n: number) => Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0");
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const clean = hex.replace("#", "");
  const num = parseInt(clean, 16);
  return {
    r: (num >> 16) & 255,
    g: (num >> 8) & 255,
    b: num & 255,
  };
}

function applyAllColorsToDOM() {
  for (const [cssVar, hexValue] of Object.entries(currentCustomColors)) {
    document.documentElement.style.setProperty(cssVar, hexValue);
  }
  localStorage.setItem("wedit_custom_colors", JSON.stringify(currentCustomColors));
}

async function registerFontInBrowser(fontFamily: string, dataUrl: string) {
  const fontFace = new FontFace(fontFamily, `url(${dataUrl})`);
  const loadedFace = await fontFace.load();
  document.fonts.add(loadedFace);

  const selectEl = document.getElementById("select-font-family") as HTMLSelectElement;
  const exists = Array.from(selectEl.options).some((opt) => opt.value === `'${fontFamily}', monospace`);
  if (!exists) {
    const opt = document.createElement("option");
    opt.value = `'${fontFamily}', monospace`;
    opt.textContent = `${fontFamily} (Font đã thêm)`;
    selectEl.appendChild(opt);
  }
  selectEl.value = `'${fontFamily}', monospace`;
  document.documentElement.style.setProperty("--editor-font", selectEl.value);
}

function initAppearanceSystem() {
  const savedColors = localStorage.getItem("wedit_custom_colors");
  if (savedColors) {
    try {
      currentCustomColors = { ...DEFAULT_DARK_COLORS, ...JSON.parse(savedColors) };
    } catch (_) {}
  }
  applyAllColorsToDOM();

  const savedFontsRaw = localStorage.getItem("wedit_custom_fonts");
  const savedFonts: CustomFontData[] = savedFontsRaw ? JSON.parse(savedFontsRaw) : [];
  savedFonts.forEach((f) => {
    registerFontInBrowser(f.font_family, f.data_url).catch(() => {});
  });

  const savedFontFamily = localStorage.getItem("wedit_selected_font");
  if (savedFontFamily) {
    document.documentElement.style.setProperty("--editor-font", savedFontFamily);
  }
  const savedFontSize = localStorage.getItem("wedit_font_size") || "14";
  document.documentElement.style.setProperty("--editor-font-size", `${savedFontSize}px`);
  (document.getElementById("slider-font-size") as HTMLInputElement).value = savedFontSize;
  document.getElementById("lbl-font-size")!.textContent = `${savedFontSize}px`;

  const targetSelect = document.getElementById("select-color-target") as HTMLSelectElement;
  const rSlider = document.getElementById("rgb-r") as HTMLInputElement;
  const gSlider = document.getElementById("rgb-g") as HTMLInputElement;
  const bSlider = document.getElementById("rgb-b") as HTMLInputElement;
  const rNum = document.getElementById("num-r") as HTMLInputElement;
  const gNum = document.getElementById("num-g") as HTMLInputElement;
  const bNum = document.getElementById("num-b") as HTMLInputElement;
  const colorWheel = document.getElementById("native-color-picker") as HTMLInputElement;
  const hexOutput = document.getElementById("hex-output")!;

  function loadTargetColorToSliders() {
    const cssVar = targetSelect.value;
    const hex = currentCustomColors[cssVar] || "#1e1e2e";
    const { r, g, b } = hexToRgb(hex);
    rSlider.value = rNum.value = String(r);
    gSlider.value = gNum.value = String(g);
    bSlider.value = bNum.value = String(b);
    colorWheel.value = hex;
    hexOutput.textContent = hex.toUpperCase();
  }

  function applyCurrentSlidersToTarget() {
    const r = parseInt(rSlider.value, 10) || 0;
    const g = parseInt(gSlider.value, 10) || 0;
    const b = parseInt(bSlider.value, 10) || 0;
    rNum.value = String(r);
    gNum.value = String(g);
    bNum.value = String(b);

    const hex = rgbToHex(r, g, b);
    colorWheel.value = hex;
    hexOutput.textContent = hex.toUpperCase();

    const cssVar = targetSelect.value;
    currentCustomColors[cssVar] = hex;
    applyAllColorsToDOM();
  }

  targetSelect.onchange = loadTargetColorToSliders;

  [rSlider, gSlider, bSlider].forEach((slider) => {
    slider.oninput = applyCurrentSlidersToTarget;
  });

  [rNum, gNum, bNum].forEach((numInput) => {
    numInput.oninput = () => {
      rSlider.value = rNum.value;
      gSlider.value = gNum.value;
      bSlider.value = bNum.value;
      applyCurrentSlidersToTarget();
    };
  });

  colorWheel.oninput = () => {
    const { r, g, b } = hexToRgb(colorWheel.value);
    rSlider.value = rNum.value = String(r);
    gSlider.value = gNum.value = String(g);
    bSlider.value = bNum.value = String(b);
    applyCurrentSlidersToTarget();
  };

  document.getElementById("btn-preset-dark")!.onclick = () => {
    currentCustomColors = { ...DEFAULT_DARK_COLORS };
    applyAllColorsToDOM();
    loadTargetColorToSliders();
  };

  document.getElementById("btn-preset-light")!.onclick = () => {
    currentCustomColors = {
      "--bg-primary": "#ffffff",
      "--bg-secondary": "#f1f3f5",
      "--bg-surface": "#e9ecef",
      "--text-main": "#212529",
      "--accent": "#1971c2",
      "--border": "#dee2e6",
    };
    applyAllColorsToDOM();
    loadTargetColorToSliders();
  };

  document.getElementById("btn-preset-amoled")!.onclick = () => {
    currentCustomColors = {
      "--bg-primary": "#000000",
      "--bg-secondary": "#0a0a0a",
      "--bg-surface": "#161616",
      "--text-main": "#e0e0e0",
      "--accent": "#00e676",
      "--border": "#262626",
    };
    applyAllColorsToDOM();
    loadTargetColorToSliders();
  };

  const fontSelect = document.getElementById("select-font-family") as HTMLSelectElement;
  fontSelect.onchange = () => {
    document.documentElement.style.setProperty("--editor-font", fontSelect.value);
    localStorage.setItem("wedit_selected_font", fontSelect.value);
  };

  document.getElementById("btn-import-font")!.onclick = async () => {
    const selectedPath = await open({
      multiple: false,
      filters: [{ name: "Fonts", extensions: ["ttf", "otf", "woff", "woff2"] }],
    });

    if (typeof selectedPath === "string") {
      try {
        const fontData = await invoke<CustomFontData>("load_custom_font", { path: selectedPath });
        await registerFontInBrowser(fontData.font_family, fontData.data_url);

        savedFonts.push(fontData);
        localStorage.setItem("wedit_custom_fonts", JSON.stringify(savedFonts));
        localStorage.setItem("wedit_selected_font", `'${fontData.font_family}', monospace`);
        alert(`Đã nạp thành công phông chữ: ${fontData.font_family}`);
      } catch (err) {
        alert(`Lỗi nạp phông chữ: ${err}`);
      }
    }
  };

  const sizeSlider = document.getElementById("slider-font-size") as HTMLInputElement;
  sizeSlider.oninput = () => {
    const size = sizeSlider.value;
    document.getElementById("lbl-font-size")!.textContent = `${size}px`;
    document.documentElement.style.setProperty("--editor-font-size", `${size}px`);
    localStorage.setItem("wedit_font_size", size);
  };

  loadTargetColorToSliders();
}

window.addEventListener("DOMContentLoaded", async () => {
  initEditor();
  initAppearanceSystem();

  const tabs = await invoke<DocumentMetadata[]>("get_all_tabs");
  if (tabs.length > 0) {
    await switchToTab(tabs[0].id);
  }

  document.getElementById("btn-new")!.onclick = async () => {
    const res = await invoke<TabContentResponse>("create_new_tab", { title: null });
    activeTabId = res.metadata.id;
    setEditorText(res.content, res.metadata.title);
    updateStatusBar(res.metadata);
    await refreshTabBar();
  };

  document.getElementById("btn-open")!.onclick = async () => {
    const selected = await open({ multiple: false });
    if (typeof selected === "string") {
      const res = await invoke<TabContentResponse>("open_local_file", { path: selected });
      activeTabId = res.metadata.id;
      setEditorText(res.content, res.metadata.title);
      updateStatusBar(res.metadata);
      await refreshTabBar();
    }
  };

  document.getElementById("btn-save")!.onclick = async () => {
    if (!activeMeta) return;
    let targetPath = activeMeta.file_path;
    if (!targetPath) {
      targetPath = await save({ defaultPath: activeMeta.title });
    }
    if (targetPath) {
      const meta = await invoke<DocumentMetadata>("save_local_file", {
        tabId: activeTabId,
        path: targetPath,
      });
      updateStatusBar(meta);
      await refreshTabBar();
    }
  };

  document.getElementById("btn-undo")!.onclick = async () => {
    try {
      const res = await invoke<TabContentResponse>("undo_action", { tabId: activeTabId });
      setEditorText(res.content, res.metadata.title);
      updateStatusBar(res.metadata);
    } catch (_) {}
  };

  document.getElementById("btn-redo")!.onclick = async () => {
    try {
      const res = await invoke<TabContentResponse>("redo_action", { tabId: activeTabId });
      setEditorText(res.content, res.metadata.title);
      updateStatusBar(res.metadata);
    } catch (_) {}
  };

  document.getElementById("btn-search-toggle")!.onclick = () => {
    document.getElementById("search-panel")!.classList.toggle("hidden");
  };

  document.getElementById("btn-find")!.onclick = async () => {
    const query = (document.getElementById("search-input") as HTMLInputElement).value;
    const caseSensitive = (document.getElementById("chk-case") as HTMLInputElement).checked;
    const useRegex = (document.getElementById("chk-regex") as HTMLInputElement).checked;

    const matches = await invoke<any[]>("search_text", {
      tabId: activeTabId,
      query,
      caseSensitive,
      useRegex,
    });
    document.getElementById("search-summary")!.textContent = `${matches.length} kết quả`;
    if (matches.length > 0) {
      const firstLine = editorView.state.doc.line(matches[0].line);
      editorView.dispatch({
        selection: { anchor: firstLine.from + matches[0].start_col - 1 },
        scrollIntoView: true,
      });
    }
  };

  document.getElementById("btn-replace-all")!.onclick = async () => {
    const query = (document.getElementById("search-input") as HTMLInputElement).value;
    const replacement = (document.getElementById("replace-input") as HTMLInputElement).value;
    const caseSensitive = (document.getElementById("chk-case") as HTMLInputElement).checked;
    const useRegex = (document.getElementById("chk-regex") as HTMLInputElement).checked;

    const res = await invoke<{ metadata: DocumentMetadata; content: string; replaced_count: number }>(
      "replace_all_text",
      { tabId: activeTabId, query, replacement, caseSensitive, useRegex }
    );
    setEditorText(res.content, res.metadata.title);
    updateStatusBar(res.metadata);
    document.getElementById("search-summary")!.textContent = `Đã thay ${res.replaced_count} vị trí`;
    await refreshTabBar();
  };

  const appearanceModal = document.getElementById("appearance-modal")!;
  document.getElementById("btn-appearance")!.onclick = () => appearanceModal.classList.remove("hidden");
  document.getElementById("btn-close-appearance")!.onclick = () => appearanceModal.classList.add("hidden");

  document.querySelectorAll("#accessory-bar button").forEach((btn) => {
    btn.addEventListener("click", () => {
      const snippet = btn.getAttribute("data-insert") || "";
      const pos = editorView.state.selection.main.head;
      editorView.dispatch({
        changes: { from: pos, insert: snippet },
        selection: { anchor: pos + snippet.length },
      });
      editorView.focus();
    });
  });

  const gdriveModal = document.getElementById("gdrive-modal")!;
  document.getElementById("btn-gdrive")!.onclick = () => gdriveModal.classList.remove("hidden");
  document.getElementById("btn-close-modal")!.onclick = () => gdriveModal.classList.add("hidden");

  document.getElementById("btn-gdrive-login")!.onclick = async () => {
    const authUrl = await invoke<string>("gdrive_get_auth_url");
    await openBrowser(authUrl);
  };

  document.getElementById("btn-gdrive-verify")!.onclick = async () => {
    const code = (document.getElementById("gdrive-code-input") as HTMLInputElement).value.trim();
    if (!code) return;
    await invoke("gdrive_authenticate", { authCode: code });
    alert("Liên kết Google Drive thành công!");
  };

  document.getElementById("btn-gdrive-refresh")!.onclick = async () => {
    const listEl = document.getElementById("gdrive-file-list")!;
    listEl.innerHTML = "<li>Đang tải danh sách từ Google Drive...</li>";
    try {
      const files = await invoke<GDriveFile[]>("gdrive_list_files", { folderId: null });
      listEl.innerHTML = "";
      files.forEach((f) => {
        const li = document.createElement("li");
        li.innerHTML = `<span>📄 ${f.name}</span> <small>${f.mimeType}</small>`;
        li.onclick = async () => {
          const res = await invoke<TabContentResponse>("gdrive_open_file", {
            fileId: f.id,
            fileName: f.name,
          });
          activeTabId = res.metadata.id;
          setEditorText(res.content, res.metadata.title);
          updateStatusBar(res.metadata);
          await refreshTabBar();
          gdriveModal.classList.add("hidden");
        };
        listEl.appendChild(li);
      });
    } catch (err) {
      listEl.innerHTML = `<li>Lỗi: ${err}</li>`;
    }
  };

  document.getElementById("btn-gdrive-upload")!.onclick = async () => {
    try {
      const meta = await invoke<DocumentMetadata>("gdrive_save_file", {
        tabId: activeTabId,
        parentFolderId: null,
      });
      updateStatusBar(meta);
      await refreshTabBar();
      alert("Đã đồng bộ file lên Google Drive thành công!");
    } catch (err) {
      alert(`Lỗi đồng bộ: ${err}`);
    }
  };
});