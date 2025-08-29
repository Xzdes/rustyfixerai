// src/modules/web_agent.rs

use super::llm_interface::AnalysisPlan;
use reqwest::Client;
use scraper::{Html, Selector};
use anyhow::Result;

// Увеличиваем лимиты, чтобы быть более настойчивыми в поиске
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
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
                .build()
                .unwrap(),
        }
    }

    /// УЛУЧШЕННЫЙ метод: использует план для выполнения нескольких поисковых запросов.
    pub async fn investigate(&self, plan: &AnalysisPlan) -> Result<String> {
        let mut all_urls = Vec::new();

        if let Some(crate_name) = &plan.involved_crate {
            println!("    -> Identified relevant crate: {}. Prioritizing its documentation.", crate_name);
            all_urls.push(format!("https://docs.rs/{}", crate_name));
        }

        for query in &plan.search_queries {
            println!("    -> Executing search query: \"{}\"", query);
            let search_url = format!("https://duckduckgo.com/html/?q={}", urlencoding::encode(query));
            match self.client.get(&search_url).send().await {
                Ok(res) => {
                    let html = res.text().await?;
                    let mut urls = self.parse_search_results(&html);
                    all_urls.append(&mut urls);
                },
                Err(e) => eprintln!("    -> Failed to execute query \"{}\": {}", query, e),
            }
        }
        
        let mut collected_context = String::new();
        let mut visited_count = 0;
        let mut visited_urls = std::collections::HashSet::new();

        for url in all_urls {
            if visited_count >= MAX_TOTAL_SITES_TO_VISIT {
                break;
            }
            if !visited_urls.insert(url.clone()) { 
                continue;
            }
            
            println!("    -> Scraping: {}", &url);
            match self.scrape_url(&url).await {
                Ok(content) => {
                    if content.len() > MIN_CONTENT_LENGTH {
                        collected_context.push_str(&format!("--- Source: {} ---\n{}\n\n", url, content));
                        visited_count += 1;
                    }
                }
                Err(e) => {
                    eprintln!("    -> Failed to scrape {}: {}", url, e);
                }
            }
        }

        Ok(collected_context)
    }

    /// Парсит HTML-страницу с результатами поиска и извлекает ссылки.
    fn parse_search_results(&self, html: &str) -> Vec<String> {
        let document = Html::parse_document(html);
        let selector = Selector::parse("a.result__a").unwrap();
        
        document.select(&selector)
            .filter_map(|element| element.value().attr("href"))
            .map(|s| s.to_string())
            .filter(|url| !url.contains("duckduckgo.com"))
            .take(MAX_RESULTS_PER_QUERY)
            .collect()
    }

    /// УЛУЧШЕННЫЙ СКРАПЕР с приоритетным извлечением блоков кода.
    async fn scrape_url(&self, url: &str) -> Result<String> {
        let html = self.client.get(url).send().await?.text().await?;
        let document = Html::parse_document(&html);
        
        let mut content = String::new();

        let code_selector = Selector::parse("pre, code").unwrap();
        for code_block in document.select(&code_selector) {
            let code_text = code_block.text().collect::<String>();
            if !code_text.trim().is_empty() {
                content.push_str("Relevant Code Example:\n```rust\n");
                content.push_str(&code_text);
                content.push_str("\n```\n\n");
            }
        }

        let main_content_selectors = "main, article, .post, .content, .entry-content, .docs-main, .answer";
        let main_selector = Selector::parse(main_content_selectors).unwrap();
        
        let main_text = if let Some(main_element) = document.select(&main_selector).next() {
            main_element.text().collect::<Vec<_>>().join(" ")
        } else {
            let body_selector = Selector::parse("body").unwrap();
            document.select(&body_selector).next().unwrap().text().collect::<Vec<_>>().join(" ")
        };
        
        content.push_str("Relevant Text:\n");
        content.push_str(&main_text.split_whitespace().collect::<Vec<_>>().join(" "));
        
        Ok(content)
    }
}