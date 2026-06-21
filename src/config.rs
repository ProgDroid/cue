use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub static_dir: String,
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub motn_api_key: Option<String>,
    pub plex_url: Option<String>,
    pub plex_token: Option<String>,
    pub region: Option<String>,
    pub sync_cron: Option<String>,
}

impl Config {
    pub fn load(get: impl Fn(&str) -> Option<String>) -> Self {
        let required = |key: &str, default: &str| get(key).unwrap_or_else(|| default.to_string());
        Self {
            bind_addr: required("BIND_ADDR", "127.0.0.1:8080"),
            database_url: required("DATABASE_URL", "sqlite:./data/cue.db"),
            static_dir: required("STATIC_DIR", "frontend/dist"),
            anthropic_api_key: get("ANTHROPIC_API_KEY"),
            openai_api_key: get("OPENAI_API_KEY"),
            motn_api_key: get("MOTN_API_KEY"),
            plex_url: get("PLEX_URL"),
            plex_token: get("PLEX_TOKEN"),
            region: get("REGION"),
            sync_cron: get("SYNC_CRON"),
        }
    }

    #[must_use]
    pub fn from_env() -> Self {
        Self::load(|k| std::env::var(k).ok())
    }

    /// On-disk path of the `SQLite` file, or `None` for an in-memory database.
    #[must_use]
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        let rest = self.database_url.strip_prefix("sqlite:")?;
        if rest.starts_with(":memory:") || rest.is_empty() {
            return None;
        }
        let path = rest.split('?').next().unwrap_or(rest);
        Some(PathBuf::from(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k: &str| map.get(k).map(|v| (*v).to_string())
    }

    #[test]
    fn defaults_apply_when_unset() {
        let cfg = Config::load(getter(HashMap::new()));
        assert_eq!(cfg.bind_addr, "127.0.0.1:8080");
        assert_eq!(cfg.database_url, "sqlite:./data/cue.db");
        assert_eq!(cfg.static_dir, "frontend/dist");
        assert!(cfg.anthropic_api_key.is_none());
    }

    #[test]
    fn env_overrides_defaults() {
        let mut m = HashMap::new();
        m.insert("BIND_ADDR", "0.0.0.0:9000");
        m.insert("ANTHROPIC_API_KEY", "sk-test");
        let cfg = Config::load(getter(m));
        assert_eq!(cfg.bind_addr, "0.0.0.0:9000");
        assert_eq!(cfg.anthropic_api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn sqlite_path_strips_scheme_and_query() {
        let cfg = Config::load(getter(HashMap::from([(
            "DATABASE_URL",
            "sqlite:./data/cue.db?mode=rwc",
        )])));
        assert_eq!(
            cfg.sqlite_path(),
            Some(std::path::PathBuf::from("./data/cue.db"))
        );
    }

    #[test]
    fn sqlite_path_is_none_for_memory() {
        let cfg = Config::load(getter(HashMap::from([("DATABASE_URL", "sqlite::memory:")])));
        assert_eq!(cfg.sqlite_path(), None);
    }
}
