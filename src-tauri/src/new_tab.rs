//! Render BlueFlame's own new-tab page as a data URL the browse webview can
//! load without any external fetch. The page has the BlueFlame brand, a
//! search box that submits to the user's chosen engine, and quick links.

use crate::brand::logo_data_url;
use crate::search::SearchEngine;
use crate::util::base64_encode;

/// One tile in the speed-dial grid. `source` is rendered as a label tag.
#[derive(Debug, Clone)]
pub struct Tile {
    pub title: String,
    pub url: String,
    pub source: TileSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileSource {
    Bookmark,
    History,
}

impl TileSource {
    fn label(self) -> &'static str {
        match self {
            TileSource::Bookmark => "[b]",
            TileSource::History => "[h]",
        }
    }
}

/// Return a `data:text/html;base64,...` URL the child webview can open.
pub fn data_url(engine: SearchEngine, tiles: &[Tile]) -> String {
    let html = render(engine, tiles);
    let encoded = base64_encode(html.as_bytes());
    format!("data:text/html;charset=utf-8;base64,{encoded}")
}

fn render(engine: SearchEngine, tiles: &[Tile]) -> String {
    let search_url = engine.search_url("__QUERY__");
    let engine_id = engine.id();
    let tiles_html = render_tiles(tiles);
    let engine_options = render_engine_options(engine);
    let engine_map = render_engine_map();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>new tab - blueflame</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;700&display=swap" rel="stylesheet">
<style>
  :root {{
    --bg: #0a0a0a;
    --bg-elev: #0d0d0d;
    --bg-raised: #141414;
    --border: #1d1d1d;
    --border-strong: #262626;
    --text: #d9e3ef;
    --text-dim: #6b7a8e;
    --text-muted: #4a5363;
    --accent: #00b3ff;
    --accent-bright: #5de0ff;
    --mono: 'JetBrains Mono', ui-monospace, Consolas, monospace;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  html, body {{ height: 100%; }}
  body {{
    font-family: var(--mono);
    background: var(--bg);
    color: var(--text);
    padding: 48px 24px 24px;
    font-size: 13px;
    line-height: 1.55;
  }}
  .term {{ max-width: 760px; margin: 0 auto; }}
  .brand-line {{
    display: flex; align-items: baseline; gap: 10px;
    color: var(--text-dim);
    font-size: 12px;
  }}
  .brand-line .logo {{
    height: 22px;
    width: auto;
    display: block;
    -webkit-user-drag: none;
    user-select: none;
    /* Aligns the logo vertically with the baseline-aligned text. */
    transform: translateY(4px);
  }}
  .brand-line .name {{ color: var(--text); font-weight: 700; font-size: 16px; }}
  .brand-line .ver {{ color: var(--text-muted); font-size: 11px; }}
  .tag {{
    color: var(--text-dim); font-size: 12px;
    margin: 6px 0 22px;
    min-height: 18px;
    white-space: pre-wrap;
  }}
  .tag .cursor {{ color: var(--accent); }}
  .cursor.blink {{ animation: blink 1s steps(2, end) infinite; }}
  @keyframes blink {{
    0%, 50% {{ opacity: 1; }}
    51%, 100% {{ opacity: 0; }}
  }}
  form {{
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 0;
    border: 1px solid var(--border-strong);
    background: var(--bg-elev);
  }}
  form:focus-within {{ border-color: var(--accent); }}
  .prompt {{
    padding: 0 10px;
    color: var(--accent);
    font-family: var(--mono);
    font-size: 13px;
    line-height: 34px;
    user-select: none;
  }}
  input[type=text] {{
    padding: 8px 4px;
    background: transparent;
    border: none;
    color: var(--text);
    font-family: var(--mono);
    font-size: 13px;
    outline: none;
    width: 100%;
  }}
  input[type=text]::placeholder {{ color: var(--text-muted); }}
  button.go {{
    padding: 8px 16px;
    background: var(--accent);
    color: #000;
    border: none;
    font-family: var(--mono);
    font-weight: 700;
    font-size: 11px;
    cursor: pointer;
  }}
  button.go:hover {{ background: var(--accent-bright); }}
  .meta {{
    margin-top: 10px;
    color: var(--text-muted);
    font-size: 11px;
    display: flex;
    align-items: center;
    gap: 6px;
  }}
  .meta .k {{ color: var(--text-dim); }}
  .meta .v {{ color: var(--accent); }}
  .engine-select {{
    background: var(--bg-elev);
    border: 1px solid var(--border-strong);
    border-radius: 0;
    color: var(--accent);
    font-family: var(--mono);
    font-size: 11px;
    padding: 2px 6px;
    cursor: pointer;
    outline: none;
  }}
  .engine-select:hover,
  .engine-select:focus {{ border-color: var(--accent); }}
  .engine-select option {{
    background: var(--bg);
    color: var(--text);
  }}
  .section {{
    margin-top: 30px;
  }}
  .section-label {{
    font-size: 11px;
    color: var(--text-dim);
    margin-bottom: 8px;
  }}
  .section-label .hash {{ color: var(--accent); }}
  .shortcuts {{
    display: flex; flex-wrap: wrap;
    gap: 0;
  }}
  .shortcuts a {{
    padding: 4px 12px;
    color: var(--text-dim);
    font-size: 12px;
    text-decoration: none;
    transition: color 120ms ease;
  }}
  .shortcuts a::before {{ content: '['; color: var(--text-muted); }}
  .shortcuts a::after {{ content: ']'; color: var(--text-muted); }}
  .shortcuts a:hover {{ color: var(--accent); }}
  .shortcuts a:hover::before,
  .shortcuts a:hover::after {{ color: var(--accent); }}
  .sd-list {{
    display: flex; flex-direction: column;
    border: 1px solid var(--border-strong);
    background: var(--bg-elev);
  }}
  .sd-row {{
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) minmax(0, 1.2fr);
    gap: 12px;
    padding: 5px 10px;
    font-size: 12px;
    color: var(--text-dim);
    text-decoration: none;
    border-bottom: 1px solid var(--border);
    transition: background 80ms ease, color 80ms ease;
  }}
  .sd-row:last-child {{ border-bottom: none; }}
  .sd-row:hover {{ background: var(--bg-raised); color: var(--accent); }}
  .sd-src {{ color: var(--text-muted); }}
  .sd-row:hover .sd-src {{ color: var(--accent); }}
  .sd-title {{
    color: var(--text);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }}
  .sd-row:hover .sd-title {{ color: var(--accent-bright); }}
  .sd-url {{
    color: var(--text-muted);
    font-size: 10px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    text-align: right;
  }}
  .sd-empty {{
    padding: 14px;
    font-size: 11px;
    color: var(--text-muted);
  }}
</style>
</head>
<body>
<main class="term">
  <div class="brand-line">
    <img class="logo" src="{logo_url}" alt="" aria-hidden="true">
    <span class="name">blueflame</span>
    <span class="ver">v0.1</span>
  </div>
  <div class="tag"><span id="tag"></span><span class="cursor blink" id="tag-cursor">_</span></div>
  <form id="f" action="javascript:void(0)" onsubmit="go(event)">
    <span class="prompt">&gt;</span>
    <input id="q" type="text" placeholder="search or enter a url" autofocus autocomplete="off" spellcheck="false">
    <button class="go" type="submit">go</button>
  </form>
  <div class="meta">
    <label class="k" for="engine-select">engine:</label>
    <select id="engine-select" class="engine-select" aria-label="search engine">{engine_options}</select>
  </div>

  <div class="section">
    <div class="section-label"><span class="hash">#</span> shortcuts</div>
    <div class="shortcuts">
      <a href="https://github.com/w1ck3ds0d4/BlueFlame">github</a>
      <a href="https://en.wikipedia.org/">wikipedia</a>
      <a href="https://news.ycombinator.com/">hacker news</a>
    </div>
  </div>

  <div class="section">
    <div class="section-label"><span class="hash">#</span> quick access</div>
    {tiles_html}
  </div>
</main>
<script>
  // Typewriter on the tagline (~35ms/char, then leaves the blinking cursor).
  const tagText = "// privacy-first browser shell";
  const tagEl = document.getElementById('tag');
  let idx = 0;
  const typer = setInterval(() => {{
    if (idx >= tagText.length) {{ clearInterval(typer); return; }}
    tagEl.textContent += tagText[idx++];
  }}, 35);


  // All engine URL templates baked in so the dropdown can override the
  // default engine just for the search initiated from this new-tab page.
  // The global default is still managed in Settings.
  const ENGINES = {engine_map};
  let activeEngine = {engine_id:?};
  const engineSelect = document.getElementById('engine-select');
  engineSelect.addEventListener('change', (e) => {{
    activeEngine = e.target.value;
  }});

  function go(e) {{
    e.preventDefault();
    const q = document.getElementById('q').value.trim();
    if (!q) return;
    if (/^https?:\/\//i.test(q)) {{ location.href = q; return; }}
    if (!q.includes(' ') && q.includes('.')) {{ location.href = 'https://' + q; return; }}
    const template = ENGINES[activeEngine] || {search_url:?};
    location.href = template.replace('__QUERY__', encodeURIComponent(q));
  }}
</script>
</body>
</html>
"#,
        search_url = search_url,
        engine_id = engine_id,
        engine_options = engine_options,
        engine_map = engine_map,
        tiles_html = tiles_html,
        logo_url = logo_data_url(),
    )
}

/// Render all `<option>` elements for the engine dropdown, marking the
/// currently-selected engine.
fn render_engine_options(current: SearchEngine) -> String {
    SearchEngine::all()
        .iter()
        .map(|e| {
            let id = e.id();
            let name = e.display_name();
            let sel = if *e == current { " selected" } else { "" };
            format!(r#"<option value="{id}"{sel}>{name}</option>"#)
        })
        .collect()
}

/// Render a JSON object literal mapping engine id -> URL template so the
/// page can swap engines without a round trip to Rust.
fn render_engine_map() -> String {
    let mut out = String::from("{");
    let mut first = true;
    for e in SearchEngine::all() {
        if !first {
            out.push(',');
        }
        first = false;
        let id = e.id();
        let template = e.search_url("__QUERY__");
        // JSON string escape - templates have no quotes or backslashes today,
        // but protect against future additions.
        let escaped = template.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("\"{id}\":\"{escaped}\""));
    }
    out.push('}');
    out
}

fn render_tiles(tiles: &[Tile]) -> String {
    if tiles.is_empty() {
        return r#"<div class="sd-empty">// no visits yet - start browsing to populate</div>"#
            .to_string();
    }
    let items: String = tiles
        .iter()
        .map(|t| {
            format!(
                r#"<a class="sd-row" href="{url}"><span class="sd-src">{src}</span><span class="sd-title">{title}</span><span class="sd-url">{url_disp}</span></a>"#,
                url = escape_attr(&t.url),
                title = escape_html(&display_title(t)),
                url_disp = escape_html(&display_url(&t.url)),
                src = t.source.label(),
            )
        })
        .collect();
    format!(r#"<div class="sd-list">{items}</div>"#)
}

fn display_title(t: &Tile) -> String {
    if !t.title.is_empty() {
        t.title.clone()
    } else {
        display_url(&t.url)
    }
}

fn display_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_else(|| url.to_string())
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    escape_html(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_html_mentions_engine_and_search_url() {
        let html = render(SearchEngine::Brave, &[]);
        assert!(html.contains("Brave Search"));
        assert!(html.contains("search.brave.com"));
    }

    #[test]
    fn data_url_has_base64_prefix() {
        let u = data_url(SearchEngine::DuckDuckGo, &[]);
        assert!(u.starts_with("data:text/html;charset=utf-8;base64,"));
    }

    #[test]
    fn speed_dial_renders_tiles() {
        let tiles = vec![
            Tile {
                title: "Rust".into(),
                url: "https://rust-lang.org".into(),
                source: TileSource::Bookmark,
            },
            Tile {
                title: "".into(),
                url: "https://github.com/w1ck3ds0d4".into(),
                source: TileSource::History,
            },
        ];
        let html = render(SearchEngine::DuckDuckGo, &tiles);
        assert!(html.contains("https://rust-lang.org"));
        assert!(html.contains(">Rust<"));
        // Empty-title tile falls back to host
        assert!(html.contains("github.com"));
    }

    #[test]
    fn speed_dial_shows_empty_hint_when_no_tiles() {
        let html = render(SearchEngine::DuckDuckGo, &[]);
        assert!(html.contains("no visits yet"));
    }

    #[test]
    fn html_escaping_blocks_injection() {
        let tiles = vec![Tile {
            title: "<script>alert(1)</script>".into(),
            url: "https://evil.example".into(),
            source: TileSource::History,
        }];
        let html = render(SearchEngine::DuckDuckGo, &tiles);
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
