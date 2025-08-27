// src/modules/web_agent.rs

use super::llm_interface::AnalysisPlan;
use reqwest::Client;
use scraper::{Html, Selector};

const SEARCH_ENGINE_URL: &str = "https://duckduckgo.com/html/?q=";
const MAX_RESULTS_TO_VISIT: usize = 3;
const MIN_CONTENT_LENGTH: usize = 300;

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

    /// УЛУЧШЕННЫЙ метод: использует план для целенаправленного поиска.
    pub async fn investigate(&self, plan: &AnalysisPlan) -> Result<String, Box<dyn std::error::Error>> {
        let mut urls_to_visit = Vec::new();

        // 1. Приоритетный поиск в документации, если крейт известен
        if let Some(crate_name) = &plan.involved_crate {
            println!("    -> Identified relevant crate: {}. Searching its documentation.", crate_name);
            // Добавляем ссылки на официальные ресурсы
            urls_to_visit.push(format!("https://docs.rs/{}", crate_name));
            urls_to_visit.push(format!("https://crates.io/crates/{}", crate_name));
        }

        // 2. Общий поиск в интернете
        let search_url = format!("{}{}", SEARCH_ENGINE_URL, &plan.search_keywords);
        let search_results_html = self.client.get(&search_url).send().await?.text().await?;
        let mut general_urls = self.parse_search_results(&search_results_html);
        urls_to_visit.append(&mut general_urls);
        
        // 3. Сбор контекста с посещением уникальных URL
        let mut collected_context = String::new();
        let mut visited_count = 0;
        let mut visited_urls = std::collections::HashSet::new();

        for url in urls_to_visit {
            if visited_count >= MAX_RESULTS_TO_VISIT {
                break;
            }
            if !visited_urls.insert(url.clone()) { // Пропускаем дубликаты
                continue;
            }
            
            println!("    -> Visiting: {}", &url);
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
            .collect()
    }

    /// Загружает URL и извлекает основной текстовый контент.
    async fn scrape_url(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        let html = self.client.get(url).send().await?.text().await?;
        let document = Html::parse_document(&html);
        
        let main_content_selectors = "main, article, .post, .content, .entry-content, .docs-main";
        let main_selector = Selector::parse(main_content_selectors).unwrap();
        
        let main_text = if let Some(main_element) = document.select(&main_selector).next() {
            main_element.text().collect::<Vec<_>>().join(" ")
        } else {
            let body_selector = Selector::parse("body").unwrap();
            document.select(&body_selector).next().unwrap().text().collect::<Vec<_>>().join(" ")
        };
        
        Ok(main_text.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}