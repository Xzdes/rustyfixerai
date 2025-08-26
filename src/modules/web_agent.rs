// src/modules/web_agent.rs

use reqwest::Client;
use scraper::{Html, Selector};

const SEARCH_ENGINE_URL: &str = "https://duckduckgo.com/html/?q=";
const MAX_RESULTS_TO_VISIT: usize = 3; // Посетим первые 3 релевантные ссылки
const MIN_CONTENT_LENGTH: usize = 300; // Минимальная длина контента для рассмотрения

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

    /// Главный метод: ищет информацию и собирает релевантный контекст.
    pub async fn investigate(&self, keywords: &str) -> Result<String, Box<dyn std::error::Error>> {
        let search_url = format!("{}{}", SEARCH_ENGINE_URL, keywords);
        let search_results_html = self.client.get(&search_url).send().await?.text().await?;
        let urls = self.parse_search_results(&search_results_html);
        
        let mut collected_context = String::new();
        let mut visited_count = 0;

        for url in urls {
            if visited_count >= MAX_RESULTS_TO_VISIT {
                break;
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
                    // Используем eprintln для вывода ошибок, это стандартная практика
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
            .filter(|url| !url.contains("duckduckgo.com")) // Исключаем рекламные и служебные ссылки
            .collect()
    }

    /// Загружает URL и извлекает основной текстовый контент.
    async fn scrape_url(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        let html = self.client.get(url).send().await?.text().await?;
        let document = Html::parse_document(&html);

        // Пробуем найти основной контент в семантических тегах
        let main_content_selectors = "main, article, .post, .content, .entry-content";
        let main_selector = Selector::parse(main_content_selectors).unwrap();
        
        let main_text = if let Some(main_element) = document.select(&main_selector).next() {
            main_element.text().collect::<Vec<_>>().join(" ")
        } else {
            // Если не нашли, берем текст из `body` как запасной вариант
            let body_selector = Selector::parse("body").unwrap();
            // .next().unwrap() здесь безопасен, т.к. у документа всегда есть body
            document.select(&body_selector).next().unwrap().text().collect::<Vec<_>>().join(" ")
        };
        
        // Очищаем текст от лишних пробелов и переносов строк
        Ok(main_text.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}