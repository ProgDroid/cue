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
    /// Browser-facing Plex base URL for building watch links; defaults to
    /// `PLEX_URL` (identical on a single-host self-hosted deployment).
    pub plex_web_url: Option<String>,
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
            // Browser-facing Plex base for watch links; defaults to PLEX_URL
            // (identical on a single-host self-hosted deployment).
            plex_web_url: get("PLEX_WEB_URL").or_else(|| get("PLEX_URL")),
            region: get("REGION"),
            sync_cron: get("SYNC_CRON"),
        }
    }

    #[must_use]
    pub fn from_env() -> Self {
        Self::load(|k| std::env::var(k).ok())
    }

    /// Validate values that otherwise only fail deep into startup (`BIND_ADDR`
    /// at `.bind()`, `SYNC_CRON` at scheduler build) — after the pool is opened
    /// and a startup sync may already be running. Returns a message naming the
    /// offending value so a bad `.env` fails fast and legibly.
    ///
    /// # Errors
    /// Returns `Err` if `BIND_ADDR` is not a resolvable `host:port` or
    /// `SYNC_CRON` is not a 6-field (seconds-precision) cron expression.
    pub fn validate(&self) -> Result<(), String> {
        use std::net::ToSocketAddrs;
        self.bind_addr.to_socket_addrs().map_err(|e| {
            format!(
                "BIND_ADDR `{}` is not a bindable host:port: {e}",
                self.bind_addr
            )
        })?;
        if let Some(cron) = &self.sync_cron {
            let fields = cron.split_whitespace().count();
            if fields != 6 {
                return Err(format!(
                    "SYNC_CRON `{cron}` must have 6 space-separated fields \
                     (sec min hour day-of-month month day-of-week), got {fields}"
                ));
            }
        }
        Ok(())
    }

    /// Directory for runtime data files (Fribb cache, etc.). Parent of the
    /// sqlite file, or `./data` for in-memory / unparsable URLs.
    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.sqlite_path()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("./data"))
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

    #[test]
    fn validate_accepts_defaults_and_six_field_cron() {
        let mut m = HashMap::new();
        m.insert("SYNC_CRON", "0 0 3 * * *");
        assert!(Config::load(getter(m)).validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_bind_addr_naming_it() {
        let mut m = HashMap::new();
        m.insert("BIND_ADDR", "not-an-address");
        let err = Config::load(getter(m)).validate().unwrap_err();
        assert!(
            err.contains("BIND_ADDR"),
            "error should name the var: {err}"
        );
    }

    #[test]
    fn validate_rejects_five_field_cron_naming_it() {
        let mut m = HashMap::new();
        m.insert("SYNC_CRON", "0 3 * * *"); // 5-field crontab, missing seconds
        let err = Config::load(getter(m)).validate().unwrap_err();
        assert!(
            err.contains("SYNC_CRON"),
            "error should name the var: {err}"
        );
    }

    #[test]
    fn plex_web_url_defaults_to_plex_url() {
        let m = HashMap::from([("PLEX_URL", "http://lan:32400")]);
        let c = Config::load(getter(m));
        assert_eq!(c.plex_web_url.as_deref(), Some("http://lan:32400"));
    }

    #[test]
    fn plex_web_url_wins_when_explicitly_set() {
        let m = HashMap::from([
            ("PLEX_URL", "http://lan:32400"),
            ("PLEX_WEB_URL", "https://plex.example.com"),
        ]);
        let c = Config::load(getter(m));
        assert_eq!(c.plex_web_url.as_deref(), Some("https://plex.example.com"));
    }
}
