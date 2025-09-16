# RustyFixerAI

RustyFixerAI — это CLI-инструмент на Rust, который анализирует ошибки сборки и тестов, автоматически находит решения с помощью LLM и применяет их. Поддерживается интеграция с [Ollama](https://ollama.com) для работы моделей локально.

## Предварительные требования

* Rust + Cargo

  ```bash
  rustup install stable
  rustup default stable
  ```
* [Ollama](https://ollama.com/download) — сервер моделей LLM.
* Загруженная модель (по умолчанию `llama3:8b`):

  ```bash
  ollama pull llama3:8b
  ```

## Локальная установка

1. Клонировать проект:

   ```bash
   git clone <URL_репозитория> rustyfixerai
   cd rustyfixerai
   ```

2. Собрать и установить бинарь глобально:

   ```bash
   cargo install --path .
   ```

   > Для обновления:

   ```bash
   cargo install --path . --force
   ```

3. Убедиться, что папка `~/.cargo/bin` (Linux/macOS) или `%USERPROFILE%\.cargo\bin` (Windows) добавлена в `PATH`.

   Проверка:

   ```bash
   rusty-fixer-ai --help
   ```

## Запуск в других проектах

1. Перейти в папку с проектом, который нужно починить:

   ```bash
   cd path/to/other-rust-project
   ```

2. Запустить инструмент:

   ```bash
   rusty-fixer-ai
   ```

### Примеры запуска

* Обычный запуск:

  ```bash
  rusty-fixer-ai
  ```
* Игнорировать локальный кэш решений:

  ```bash
  rusty-fixer-ai --no-cache
  ```
* Исправить ошибки и затем предупреждения:

  ```bash
  rusty-fixer-ai --fix-warnings
  ```

> Важно: запускать из корня проекта (где находится `Cargo.toml`).

## Переменные окружения

* Указать кастомный URL Ollama:

  ```bash
  export OLLAMA_BASE_URL="http://127.0.0.1:11434"
  ```
* Указать модель:

  ```bash
  export OLLAMA_MODEL="llama3:8b"
  ```

Проверить сервер:

```bash
curl http://127.0.0.1:11434/api/tags
```

## Удаление

Если потребуется удалить бинарь:

```bash
cargo uninstall rusty-fixer-ai
