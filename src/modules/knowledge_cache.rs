use rusqlite::{Connection, Result};

const DB_FILE: &str = ".rusty_fixer_cache.db";

pub struct KnowledgeCache {
    conn: Connection,
}

impl KnowledgeCache {
    pub fn new() -> Result<Self> {
        let conn = Connection::open(DB_FILE)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS solutions(
                signature TEXT PRIMARY KEY,
                full_source TEXT NOT NULL
            );",
        )?;
        Ok(Self { conn })
    }

    pub fn lookup(&self, signature: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT full_source FROM solutions WHERE signature=?1")?;
        let mut rows = stmt.query([signature])?;
        if let Some(row) = rows.next()? {
            let code: String = row.get(0)?;
            return Ok(Some(code));
        }
        Ok(None)
    }

    pub fn store(&self, signature: &str, code: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO solutions(signature, full_source) VALUES(?1, ?2)",
            (signature, code),
        )?;
        Ok(())
    }
}
