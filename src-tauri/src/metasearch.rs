//! Minimal metasearch: hit the HTML endpoint of a privacy engine, parse the
//! results, render our own page. Current coverage is DuckDuckGo only;
//! more engines + cross-engine dedup can be added in a follow-up.

use anyhow::Context;
use scraper::{Html, Selector};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
}

/// Run a metasearch query and return up to `max_results` results.
pub async fn search(query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
    let q = urlencoding_encode(query);
    let endpoint = format!("https://html.duckduckgo.com/html/?q={q}");

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; BlueFlame-Metasearch/0.1)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("building http client")?;

    let body = client
        .get(&endpoint)
        .send()
        .await
        .context("ddg request")?
        .text()
        .await
        .context("ddg body")?;

    Ok(parse_ddg_html(&body, max_results))
}

/// Parse DuckDuckGo's HTML endpoint output. Exposed for unit tests with fixture.
fn parse_ddg_html(body: &str, max_results: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let result_sel = Selector::parse("div.result").unwrap();
    let title_sel = Selector::parse("a.result__a").unwrap();
    let snippet_sel = Selector::parse("a.result__snippet, .result__snippet").unwrap();

    let mut out = Vec::with_capacity(max_results);
    for node in doc.select(&result_sel) {
        if out.len() >= max_results {
            break;
        }
        let Some(a) = node.select(&title_sel).next() else {
            continue;
        };
        let title = a.text().collect::<String>().trim().to_string();
        let raw_href = a.value().attr("href").unwrap_or("").to_string();
        let url = clean_ddg_url(&raw_href);
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = node
            .select(&snippet_sel)
            .next()
            .map(|s| s.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url,
            snippet,
            source: "DuckDuckGo".into(),
        });
    }
    out
}

/// DuckDuckGo HTML endpoint wraps real URLs behind an `uddg=` redirect param.
/// Unwrap so results point directly at the destination.
fn clean_ddg_url(raw: &str) -> String {
    let s = if let Some(rest) = raw.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        raw.to_string()
    };
    if let Ok(parsed) = url::Url::parse(&s) {
        if let Some(uddg) = parsed.query_pairs().find(|(k, _)| k == "uddg") {
            return uddg.1.into_owned();
        }
    }
    s
}

/// Render results as a self-contained HTML page the browse webview can load.
pub fn results_page(query: &str, results: &[SearchResult]) -> String {
    let escaped_q = escape_html(query);
    let items: String = results
        .iter()
        .map(|r| {
            format!(
                r#"<li class="r">
  <a class="r-t" href="{url}" rel="noopener">{title}</a>
  <div class="r-u">{url_display}</div>
  <p class="r-s">{snippet}</p>
</li>"#,
                url = escape_attr(&r.url),
                title = escape_html(&r.title),
                url_display = escape_html(&r.url),
                snippet = escape_html(&r.snippet),
            )
        })
        .collect();

    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{escaped_q} - blueflame search</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;700&display=swap" rel="stylesheet">
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{
  font-family:'JetBrains Mono',ui-monospace,Consolas,monospace;
  background:#0a0a0a;color:#d9e3ef;
  padding:24px;max-width:800px;margin:0 auto;
  font-size: 14px;line-height:1.55;
}}
.hdr{{
  display:flex;align-items:baseline;gap:10px;
  margin-bottom:18px;padding-bottom:10px;
  border-bottom:1px solid #262626;
}}
.hdr .logo{{ height:22px; width:auto; display:block; transform:translateY(4px); -webkit-user-drag:none; user-select:none; }}
.hdr h1{{ font-size: 15px; font-weight:700; color:#d9e3ef; }}
.hdr .q{{
  color:#6b7a8e; font-size: 12px; margin-left:auto;
}}
.hdr .q::before{{ content:'q: '; color:#4a5363; }}
ul{{ list-style:none; }}
.r{{
  margin:0 0 2px; padding:8px 10px;
  background:transparent;
  border:1px solid #262626;
  transition:background 80ms ease, border-color 80ms ease;
}}
.r:hover{{ background:#141414; border-color:rgba(0,179,255,0.35); }}
.r-t{{
  color:#5de0ff; font-size: 14px;
  text-decoration:none; font-weight:500;
}}
.r-t:hover{{ text-decoration:underline; }}
.r-u{{
  color:#4a5363; font-size: 11px;
  margin:2px 0 4px; word-break:break-all;
}}
.r-u::before{{ content:'> '; color:#6b7a8e; }}
.r-s{{ color:#d9e3ef; font-size: 13px; }}
.src{{
  color:#4a5363; font-size: 12px;
  margin-top:18px; text-align:left;
}}
.src::before{{ content:'// '; color:#00b3ff; }}
.empty{{
  color:#6b7a8e; font-size: 12px;
  padding:28px 0; text-align:left;
}}
.empty::before{{ content:'// '; color:#00b3ff; }}
</style></head><body>
<div class="hdr">
  <img class="logo" src="{logo_url}" alt="" aria-hidden="true">
  <h1>blueflame / search</h1>
  <span class="q">{escaped_q}</span>
</div>
<ul>{items}</ul>
{footer}
</body></html>"#,
        escaped_q = escaped_q,
        logo_url = crate::brand::logo_data_url(),
        items = if items.is_empty() {
            r#"<li class="empty">NO RESULTS</li>"#.to_string()
        } else {
            items
        },
        footer = if results.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="src">{} results / source: duckduckgo html</div>"#,
                results.len()
            )
        },
    )
}

/// Render results into a data URL the browse webview can open directly.
pub fn results_data_url(query: &str, results: &[SearchResult]) -> String {
    let html = results_page(query, results);
    let b64 = crate::util::base64_encode(html.as_bytes());
    format!("data:text/html;charset=utf-8;base64,{b64}")
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
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

    const FIXTURE: &str = r##"
<html><body>
<div class="result">
  <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=1">Example Page</a>
  <a class="result__snippet" href="#">This is an example snippet.</a>
</div>
<div class="result">
  <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ffoo.test%2F">Foo Test</a>
  <div class="result__snippet">Snippet for foo.</div>
</div>
<div class="result">
  <a class="result__a" href="">Empty URL</a>
  <div class="result__snippet">should be skipped</div>
</div>
</body></html>
"##;

    #[test]
    fn parses_ddg_html_fixture() {
        let r = parse_ddg_html(FIXTURE, 10);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].title, "Example Page");
        assert_eq!(r[0].url, "https://example.com/page");
        assert!(r[0].snippet.contains("example snippet"));
        assert_eq!(r[1].url, "https://foo.test/");
    }

    #[test]
    fn parse_respects_max_results() {
        let r = parse_ddg_html(FIXTURE, 1);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn clean_ddg_url_unwraps_redirect() {
        let out = clean_ddg_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F&rut=1");
        assert_eq!(out, "https://example.com/");
    }

    #[test]
    fn escape_html_escapes_specials() {
        assert_eq!(
            escape_html(r#"<a href="x">&"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;"
        );
    }

    #[test]
    fn results_page_renders_query_and_results() {
        let r = vec![SearchResult {
            title: "Hello".into(),
            url: "https://example.com".into(),
            snippet: "Snip".into(),
            source: "DuckDuckGo".into(),
        }];
        let page = results_page("what is rust", &r);
        assert!(page.contains("what is rust"));
        assert!(page.contains("https://example.com"));
        assert!(page.contains("Hello"));
    }
}
