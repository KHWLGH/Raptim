use serde::Serialize;
use std::path::Path;

#[derive(Clone, Serialize)]
pub struct RenderedEntry {
    pub id: String,
    pub group: String,
    pub title: String,
    pub description: String,
    pub light_rel: Option<String>,
    pub dark_rel: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize)]
pub struct GalleryMeta {
    pub app_version: String,
    pub lang: String,
    pub preset: String,
    pub theme: String,
    pub frame: String,
    pub scale: f32,
    pub overview_light: Option<String>,
    pub overview_dark: Option<String>,
}

#[derive(Serialize)]
struct Manifest<'a> {
    meta: &'a GalleryMeta,
    scenes: &'a [RenderedEntry],
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn write_gallery(
    out_dir: &Path,
    entries: &[RenderedEntry],
    meta: &GalleryMeta,
) -> Result<(), String> {
    let manifest = Manifest {
        meta,
        scenes: entries,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("序列化 manifest.json 失败：{e}"))?;
    std::fs::write(out_dir.join("manifest.json"), manifest_json)
        .map_err(|e| format!("写入 manifest.json 失败：{e}"))?;

    let html = build_html(entries, meta);
    std::fs::write(out_dir.join("index.html"), html)
        .map_err(|e| format!("写入 index.html 失败：{e}"))?;
    Ok(())
}

fn build_html(entries: &[RenderedEntry], meta: &GalleryMeta) -> String {
    let en = meta.lang == "en";
    let t = |zh, en_s| if en { en_s } else { zh };
    let has_light = entries.iter().any(|e| e.light_rel.is_some());
    let has_dark = entries.iter().any(|e| e.dark_rel.is_some());
    let default_mode = if has_light && has_dark {
        "split"
    } else if has_dark {
        "dark"
    } else {
        "light"
    };

    let mut overview_html = String::new();
    if meta.overview_light.is_some() || meta.overview_dark.is_some() {
        let overview_heading = t("总览拼图", "Overview Grid");
        let light_alt = t("浅色主题总览", "Light theme overview");
        let dark_alt = t("深色主题总览", "Dark theme overview");
        overview_html.push_str(&format!(
            r#"<section class="overview-section" id="overviewSection"><h2>{overview_heading}</h2><div class="overview-grid">"#
        ));
        if let Some(light) = &meta.overview_light {
            let esc = escape_html(light);
            overview_html.push_str(&format!(
                r#"<figure class="overview-card" data-theme="light"><img src="{esc}" alt="{light_alt}" onclick="openLightbox('{esc}', '{light_alt}')"/><figcaption><span>{light_alt}</span><code>{esc}</code></figcaption></figure>"#
            ));
        }
        if let Some(dark) = &meta.overview_dark {
            let esc = escape_html(dark);
            overview_html.push_str(&format!(
                r#"<figure class="overview-card" data-theme="dark"><img src="{esc}" alt="{dark_alt}" onclick="openLightbox('{esc}', '{dark_alt}')"/><figcaption><span>{dark_alt}</span><code>{esc}</code></figcaption></figure>"#
            ));
        }
        overview_html.push_str("</div></section>");
    }

    let mut cards_html = String::new();
    for entry in entries {
        let id = escape_html(&entry.id);
        let group = escape_html(&entry.group);
        let title = escape_html(&entry.title);
        let desc = escape_html(&entry.description);
        let light = entry.light_rel.as_deref().unwrap_or("");
        let dark = entry.dark_rel.as_deref().unwrap_or("");
        let light_esc = escape_html(light);
        let dark_esc = escape_html(dark);
        let primary_rel = if !light.is_empty() { light } else { dark };
        let primary_esc = escape_html(primary_rel);
        let md_snippet = if !light.is_empty() && !dark.is_empty() {
            if en {
                format!("![{title} (Light)]({light}) / ![{title} (Dark)]({dark})")
            } else {
                format!("![{title}]({light}) / ![{title}（深色）]({dark})")
            }
        } else {
            format!("![{title}]({primary_rel})")
        };
        let md_esc = escape_html(&md_snippet);

        let mut shots_html = String::new();
        let light_badge = t("浅色", "Light");
        let dark_badge = t("深色", "Dark");
        let light_title = if en {
            format!("{title} (Light)")
        } else {
            format!("{title}（浅色）")
        };
        let dark_title = if en {
            format!("{title} (Dark)")
        } else {
            format!("{title}（深色）")
        };
        if !light.is_empty() {
            shots_html.push_str(&format!(
                r#"<div class="shot" data-shot="light" onclick="openLightbox('{light_esc}', '{light_title}')"><span class="shot-badge">{light_badge}</span><img loading="lazy" src="{light_esc}" alt="{light_title}"/></div>"#
            ));
        }
        if !dark.is_empty() {
            shots_html.push_str(&format!(
                r#"<div class="shot shot-dark" data-shot="dark" onclick="openLightbox('{dark_esc}', '{dark_title}')"><span class="shot-badge">{dark_badge}</span><img loading="lazy" src="{dark_esc}" alt="{dark_title}"/></div>"#
            ));
        }

        let copy_path_label = t("复制路径", "Copy path");
        let copy_path_toast = t("已复制路径", "Path copied");
        let copy_md_label = t("复制 Markdown", "Copy Markdown");
        let copy_md_toast = t("已复制 Markdown", "Markdown copied");
        cards_html.push_str(&format!(
            r#"<article class="card" data-group="{group}" data-id="{id}">
  <div class="card-head">
    <div>
      <div class="card-title-row">
        <h3>{title}</h3>
        <span class="pill">{group}</span>
      </div>
      <p class="card-desc">{desc}</p>
    </div>
    <div class="card-meta"><code>{id}</code><span>{}×{} px</span></div>
  </div>
  <div class="shots">{shots_html}</div>
  <div class="card-actions">
    <button type="button" onclick="copyText('{primary_esc}', '{copy_path_toast}')">{copy_path_label}</button>
    <button type="button" onclick="copyText('{md_esc}', '{copy_md_toast}')">{copy_md_label}</button>
  </div>
</article>"#,
            entry.width, entry.height
        ));
    }

    let version = escape_html(&meta.app_version);
    let preset = escape_html(&meta.preset);
    let frame = escape_html(&meta.frame);
    let theme = escape_html(&meta.theme);
    let scale = format!("{:.2}", meta.scale);
    let html_lang = if en { "en" } else { "zh-CN" };
    let page_title = t("Raptim 界面预览图库", "Raptim UI Preview Gallery");
    let font_family = if en {
        r#""Segoe UI", system-ui, -apple-system, sans-serif"#
    } else {
        r#""Microsoft YaHei UI", "Segoe UI", system-ui, -apple-system, sans-serif"#
    };
    let meta_version = t("版本", "Version");
    let meta_preset = t("预设", "Preset");
    let meta_theme = t("主题", "Theme");
    let meta_frame = t("边框", "Frame");
    let meta_scale = t("缩放", "Scale");
    let meta_scenes = if en {
        format!("{} scenes", entries.len())
    } else {
        format!("共 {} 个场景", entries.len())
    };
    let all_group = t("全部", "All");
    let main_group = t("主窗口", "Main Window");
    let editor_group = t("编辑窗口", "Editor");
    let settings_group = t("设置窗口", "Settings");
    let split_label = t("并排对比", "Split");
    let light_label = t("浅色", "Light");
    let dark_label = t("深色", "Dark");
    let prev_label = t("上一张", "Previous");
    let next_label = t("下一张", "Next");
    let close_label = t("关闭 (Esc)", "Close (Esc)");
    let toast_sep = if en { ": " } else { "：" };

    format!(
        r#"<!DOCTYPE html>
<html lang="{html_lang}">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>{page_title}</title>
<style>
:root {{
  --bg: #f5f7fb;
  --surface: #ffffff;
  --surface-alt: #eef2f7;
  --text: #111827;
  --muted: #4b5563;
  --border: #dde3ec;
  --accent: #005fb8;
  --accent-soft: #e6f0fa;
  --well-light: repeating-conic-gradient(#f1f4f8 0% 25%, #ffffff 0% 50%) 50% / 16px 16px;
  --well-dark: repeating-conic-gradient(#1b2029 0% 25%, #232934 0% 50%) 50% / 16px 16px;
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  font-family: {font_family};
  background: var(--bg);
  color: var(--text);
  line-height: 1.5;
}}
header {{
  position: sticky;
  top: 0;
  z-index: 20;
  backdrop-filter: blur(10px);
  background: rgba(245, 247, 251, 0.9);
  border-bottom: 1px solid var(--border);
}}
.header-inner {{
  max-width: 1440px;
  margin: 0 auto;
  padding: 16px 24px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}}
.brand h1 {{
  margin: 0;
  font-size: 20px;
  font-weight: 700;
}}
.brand-meta {{
  margin-top: 4px;
  font-size: 12px;
  color: var(--muted);
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}}
.controls {{
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  align-items: center;
}}
.seg {{
  display: inline-flex;
  background: var(--surface-alt);
  padding: 3px;
  border-radius: 8px;
  border: 1px solid var(--border);
  gap: 2px;
}}
.seg button {{
  border: 0;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 13px;
  padding: 6px 12px;
  border-radius: 6px;
  cursor: pointer;
}}
.seg button.active {{
  background: var(--surface);
  color: var(--accent);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0,0,0,0.06);
}}
main {{
  max-width: 1440px;
  margin: 0 auto;
  padding: 24px;
}}
.overview-section {{
  margin-bottom: 28px;
}}
.overview-section h2 {{
  margin: 0 0 12px;
  font-size: 16px;
}}
.overview-grid {{
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
  gap: 16px;
}}
.overview-card {{
  margin: 0;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  overflow: hidden;
}}
.overview-card img {{
  display: block;
  width: 100%;
  height: auto;
  cursor: zoom-in;
}}
.overview-card figcaption {{
  padding: 10px 14px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12px;
  color: var(--muted);
  border-top: 1px solid var(--border);
}}
.cards {{
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(460px, 1fr));
  gap: 20px;
}}
.card {{
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}}
.card.hidden {{ display: none; }}
.card-head {{
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
}}
.card-title-row {{
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}}
.card-title-row h3 {{
  margin: 0;
  font-size: 16px;
}}
.pill {{
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}}
.card-desc {{
  margin: 6px 0 0;
  font-size: 13px;
  color: var(--muted);
}}
.card-meta {{
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}}
code {{
  font-family: Consolas, monospace;
  background: var(--surface-alt);
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 12px;
}}
.shots {{
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 12px;
}}
.shot {{
  position: relative;
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 12px;
  background: var(--well-light);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: zoom-in;
  min-height: 220px;
}}
.shot.shot-dark {{
  background: var(--well-dark);
}}
.shot img {{
  max-width: 100%;
  max-height: 380px;
  height: auto;
  display: block;
}}
.shot-badge {{
  position: absolute;
  top: 8px;
  left: 8px;
  font-size: 11px;
  padding: 2px 7px;
  border-radius: 6px;
  background: rgba(17, 24, 39, 0.72);
  color: #fff;
}}
body[data-view="light"] .shot[data-shot="dark"],
body[data-view="dark"] .shot[data-shot="light"],
body[data-view="light"] .overview-card[data-theme="dark"],
body[data-view="dark"] .overview-card[data-theme="light"] {{
  display: none;
}}
.card-actions {{
  display: flex;
  gap: 8px;
  justify-content: flex-end;
}}
.card-actions button {{
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-size: 12px;
  padding: 5px 10px;
  border-radius: 6px;
  cursor: pointer;
}}
.card-actions button:hover {{
  border-color: var(--accent);
  color: var(--accent);
}}
.lightbox {{
  position: fixed;
  inset: 0;
  z-index: 50;
  background: rgba(10, 14, 22, 0.86);
  display: none;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px;
}}
.lightbox.open {{ display: flex; }}
.lightbox img {{
  max-width: 92vw;
  max-height: 82vh;
  object-fit: contain;
}}
.lightbox-bar {{
  margin-top: 12px;
  color: #f3f4f6;
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 14px;
}}
.lightbox-bar button {{
  border: 1px solid rgba(255,255,255,0.25);
  background: rgba(255,255,255,0.1);
  color: #fff;
  padding: 6px 12px;
  border-radius: 6px;
  cursor: pointer;
}}
.toast {{
  position: fixed;
  bottom: 20px;
  right: 20px;
  background: #111827;
  color: #fff;
  padding: 8px 14px;
  border-radius: 8px;
  font-size: 13px;
  opacity: 0;
  transition: opacity 150ms ease;
  pointer-events: none;
  z-index: 60;
}}
.toast.show {{ opacity: 1; }}
@media (max-width: 560px) {{
  .cards {{ grid-template-columns: 1fr; }}
}}
</style>
</head>
<body data-view="{default_mode}">
<header>
  <div class="header-inner">
    <div class="brand">
      <h1>{page_title}</h1>
      <div class="brand-meta">
        <span>{meta_version} v{version}</span>
        <span>{meta_preset} {preset}</span>
        <span>{meta_theme} {theme}</span>
        <span>{meta_frame} {frame}</span>
        <span>{meta_scale} {scale}x</span>
        <span>{meta_scenes}</span>
      </div>
    </div>
    <div class="controls">
      <div class="seg" id="groupTabs">
        <button type="button" class="active" data-group="{all_group}">{all_group}</button>
        <button type="button" data-group="{main_group}">{main_group}</button>
        <button type="button" data-group="{editor_group}">{editor_group}</button>
        <button type="button" data-group="{settings_group}">{settings_group}</button>
      </div>
      <div class="seg" id="themeTabs">
        <button type="button" data-view="split">{split_label}</button>
        <button type="button" data-view="light">{light_label}</button>
        <button type="button" data-view="dark">{dark_label}</button>
      </div>
    </div>
  </div>
</header>
<main>
  {overview_html}
  <section class="cards" id="cards">{cards_html}</section>
</main>
<div class="lightbox" id="lightbox" onclick="if (event.target === this) closeLightbox()">
  <img id="lightboxImg" src="" alt="" />
  <div class="lightbox-bar">
    <button type="button" onclick="stepLightbox(-1)">{prev_label}</button>
    <span id="lightboxTitle"></span>
    <button type="button" onclick="stepLightbox(1)">{next_label}</button>
    <button type="button" onclick="closeLightbox()">{close_label}</button>
  </div>
</div>
<div class="toast" id="toast"></div>
<script>
const cards = Array.from(document.querySelectorAll('.card'));
const groupButtons = Array.from(document.querySelectorAll('#groupTabs button'));
const themeButtons = Array.from(document.querySelectorAll('#themeTabs button'));
const ALL_GROUP = '{all_group}';
let activeGroup = ALL_GROUP;
let activeView = document.body.dataset.view;

function syncButtons() {{
  groupButtons.forEach(b => b.classList.toggle('active', b.dataset.group === activeGroup));
  themeButtons.forEach(b => b.classList.toggle('active', b.dataset.view === activeView));
  document.body.dataset.view = activeView;
  cards.forEach(card => {{
    const match = activeGroup === ALL_GROUP || card.dataset.group === activeGroup;
    card.classList.toggle('hidden', !match);
  }});
}}
groupButtons.forEach(btn => btn.addEventListener('click', () => {{
  activeGroup = btn.dataset.group;
  syncButtons();
}}));
themeButtons.forEach(btn => btn.addEventListener('click', () => {{
  activeView = btn.dataset.view;
  syncButtons();
}}));
syncButtons();

function visibleShots() {{
  return Array.from(document.querySelectorAll('.card:not(.hidden) .shot'))
    .filter(el => el.offsetParent !== null)
    .map(el => {{
      const img = el.querySelector('img');
      return {{ src: img.getAttribute('src'), title: img.getAttribute('alt') }};
    }});
}}
let lightboxList = [];
let lightboxIndex = 0;
function openLightbox(src, title) {{
  lightboxList = visibleShots();
  lightboxIndex = Math.max(0, lightboxList.findIndex(item => item.src === src));
  document.getElementById('lightboxImg').src = src;
  document.getElementById('lightboxTitle').textContent = title;
  document.getElementById('lightbox').classList.add('open');
}}
function closeLightbox() {{
  document.getElementById('lightbox').classList.remove('open');
}}
function stepLightbox(delta) {{
  if (!lightboxList.length) return;
  lightboxIndex = (lightboxIndex + delta + lightboxList.length) % lightboxList.length;
  const item = lightboxList[lightboxIndex];
  document.getElementById('lightboxImg').src = item.src;
  document.getElementById('lightboxTitle').textContent = item.title;
}}
document.addEventListener('keydown', e => {{
  if (!document.getElementById('lightbox').classList.contains('open')) return;
  if (e.key === 'Escape') closeLightbox();
  if (e.key === 'ArrowLeft') stepLightbox(-1);
  if (e.key === 'ArrowRight') stepLightbox(1);
}});
let toastTimer = null;
function showToast(msg) {{
  const el = document.getElementById('toast');
  el.textContent = msg;
  el.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('show'), 1800);
}}
async function copyText(text, label) {{
  try {{
    await navigator.clipboard.writeText(text);
    showToast(label + '{toast_sep}' + text);
  }} catch (_) {{
    showToast(text);
  }}
}}
</script>
</body>
</html>
"#
    )
}
