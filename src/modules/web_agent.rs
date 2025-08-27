// src/modules/web_agent.rs

use super::llm_interface::AnalysisPlan;
use reqwest::Client;
use scraper::{Html, Selector};

// Увеличиваем лимиты, чтобы быть более настойчивыми в поиске
const MAX_RESULTS_PER_QUERY: usize = 5; // Сколько ссылок брать с одной страницы поисковика
const MAX_TOTAL_SITES_TO_VISIT: usize = 5; // Общий лимит на количество посещаемых сайтов
const MIN_CONTENT_LENGTH: usize = 200; // Минимальная длина контента, чтобы считать его полезным

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
    pub async fn investigate(&self, plan: &AnalysisPlan) -> Result<String, Box<dyn std::error::Error>> {
        let mut all_urls = Vec::new();

        // 1. Приоритетный поиск в документации, если ИИ определил связанный крейт
        if let Some(crate_name) = &plan.involved_crate {
            println!("    -> Identified relevant crate: {}. Prioritizing its documentation.", crate_name);
            all_urls.push(format!("https://docs.rs/{}", crate_name));
        }

        // 2. Выполняем ВСЕ поисковые запросы, сгенерированные ИИ
        for query in &plan.search_queries {
            println!("    -> Executing search query: \"{}\"", query);
            let search_url = format!("https://duckduckgo.com/html/?q={}", query);
            match self.client.get(&search_url).send().await {
                Ok(res) => {
                    let html = res.text().await?;
                    let mut urls = self.parse_search_results(&html);
                    all_urls.append(&mut urls);
                },
                Err(e) => eprintln!("    -> Failed to execute query \"{}\": {}", query, e),
            }
        }
        
        // 3. Сбор контекста с посещением уникальных URL
        let mut collected_context = String::new();
        let mut visited_count = 0;
        let mut visited_urls = std::collections::HashSet::new();

        for url in all_urls {
            if visited_count >= MAX_TOTAL_SITES_TO_VISIT {
                break;
            }
            // Пропускаем дубликаты, чтобы не посещать один и тот же сайт дважды
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

    /// УЛУЧШЕННЫЙ СКРАПЕР с приоритетным извлечением блоков кода ("охота за кодом").
    async fn scrape_url(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        let html = self.client.get(url).send().await?.text().await?;
        let document = Html::parse_document(&html);
        
        let mut content = String::new();

        // Приоритет №1: Ищем и извлекаем все блоки кода
        let code_selector = Selector::parse("pre, code").unwrap();
        for code_block in document.select(&code_selector) {
            let code_text = code_block.text().collect::<String>();
            // Добавляем только непустые блоки кода
            if !code_text.trim().is_empty() {
                content.push_str("Relevant Code Example:\n```rust\n");
                content.push_str(&code_text);
                content.push_str("\n```\n\n");
            }
        }

        // Приоритет №2: Ищем основной текстовый контент
        let main_content_selectors = "main, article, .post, .content, .entry-content, .docs-main, .answer";
        let main_selector = Selector::parse(main_content_selectors).unwrap();
        
        let main_text = if let Some(main_element) = document.select(&main_selector).next() {
            main_element.text().collect::<Vec<_>>().join(" ")
        } else {
            // Запасной вариант - взять все из body
            let body_selector = Selector::parse("body").unwrap();
            document.select(&body_selector).next().unwrap().text().collect::<Vec<_>>().join(" ")
        };
        
        content.push_str("Relevant Text:\n");
        content.push_str(&main_text.split_whitespace().collect::<Vec<_>>().join(" "));
        
        Ok(content)
    }
}