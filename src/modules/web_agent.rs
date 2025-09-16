use super::llm_interface::AnalysisPlan;
use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};

const MAX_RESULTS_PER_QUERY: usize = 5;
const MAX_TOTAL_SITES_TO_VISIT: usize = 5;
const MIN_CONTENT_LENGTH: usize = 200;

pub struct WebAgent {
    client: Client,
}

impl WebAgent {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0 Safari/537.36")
                .build()
                .unwrap(),
        }
    }

    pub async fn investigate(&self, plan: &AnalysisPlan) -> Result<String> {
        let mut all_urls = Vec::new();

        if let Some(crate_name) = &plan.involved_crate {
            all_urls.push(format!("https://docs.rs/{}", crate_name));
        }

        for query in &plan.search_queries {
            let search_url = format!("https://duckduckgo.com/html/?q={}", urlencoding::encode(query));
            let html = self.client.get(&search_url).send().await?
                .text().await?;
            let mut urls = Self::parse_search_results(&html);
            urls.truncate(MAX_RESULTS_PER_QUERY);
            all_urls.extend(urls);
        }

        let mut collected = String::new();
        let mut visited = 0usize;
        let mut seen = std::collections::HashSet::new();

        for url in all_urls {
            if visited >= MAX_TOTAL_SITES_TO_VISIT { break; }
            if !seen.insert(url.clone()) { continue; }

            match self.scrape_url(&url).await {
                Ok(text) if text.len() >= MIN_CONTENT_LENGTH => {
                    collected.push_str(&format!("--- Source: {} ---\n{}\n\n", url, text));
                    visited += 1;
                }
                Ok(_) => {}
                Err(e) => eprintln!("    -> scrape failed {}: {e}", url),
            }
        }

        Ok(collected)
    }

    fn parse_search_results(html: &str) -> Vec<String> {
        let doc = Html::parse_document(html);
        let selector = Selector::parse("a.result__a, a.result__url, a.result__title").unwrap();
        let mut urls = Vec::new();
        for el in doc.select(&selector) {
            if let Some(href) = el.value().attr("href") {
                if href.starts_with("http") {
                    urls.push(href.to_string());
                }
            }
        }
        urls
    }

    async fn scrape_url(&self, url: &str) -> Result<String> {
        let resp = self.client.get(url).send().await
            .with_context(|| format!("fetch {}", url))?;
        let text = resp.text().await?;
        let doc = Html::parse_document(&text);
        let sel = Selector::parse("article, main, pre, code, p, li").unwrap();
        let mut buf = String::new();
        for el in doc.select(&sel) {
            let t = el.text().collect::<Vec<_>>().join(" ");
            if !t.trim().is_empty() {
                buf.push_str(t.trim());
                buf.push('\n');
            }
        }
        Ok(buf)
    }
}
