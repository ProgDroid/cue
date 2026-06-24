//! Watch-at-source redirect handler (`GET /api/titles/{id}/watch/{service}`).
//!
//! 302s to the title's page on Plex (self-hosted /web) or a streaming service
//! (MOTN link). Real URLs — machineId, internal Plex base, MOTN links — never
//! enter any client payload.

use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

use crate::models::Service;

/// Browser-facing Plex base URL for building watch links. Held in app data so
/// the internal Plex address never reaches the client.
#[derive(Clone)]
pub struct WatchConfig {
    pub plex_web_url: Option<String>,
}

/// Build the Plex web deep link for an item on the self-hosted server.
#[must_use]
pub fn plex_web_url(base: &str, machine_id: &str, rating_key: &str) -> String {
    format!(
        "{}/web/index.html#!/server/{}/details?key=%2Flibrary%2Fmetadata%2F{}",
        base.trim_end_matches('/'),
        machine_id,
        rating_key
    )
}

/// Defense-in-depth: a MOTN link comes from an external feed, so only redirect
/// to `https://` on the service's own domain (prevents an open-redirect pivot).
#[must_use]
pub fn is_allowed_motn_link(service: Service, url: &str) -> bool {
    let expected = match service {
        Service::Crunchyroll => "crunchyroll.com",
        Service::Disney => "disneyplus.com",
        // Plex watch links are built server-side from trusted config — never
        // stored as a MOTN link, so this branch is always disallowed.
        Service::Plex => return false,
    };
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str()
                .is_some_and(|h| h == expected || h.ends_with(&format!(".{expected}")))
    })
}

pub async fn redirect(
    path: web::Path<(i64, String)>,
    pool: web::Data<SqlitePool>,
    cfg: web::Data<WatchConfig>,
) -> impl Responder {
    let (id, service_str) = path.into_inner();
    let Some(service) = Service::parse(&service_str) else {
        return HttpResponse::BadRequest().finish();
    };

    let location = match service {
        Service::Plex => {
            // `fetch_optional` of `SELECT col` typed as `Option<String>` yields
            // `Result<Option<Option<String>>>` — outer = row exists, inner =
            // column NULL. `.ok().flatten().flatten()` collapses to `Option<String>`.
            let rating_key = sqlx::query_scalar::<_, Option<String>>(
                "SELECT plex_rating_key FROM titles WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten()
            .flatten();
            let machine_id = crate::db::app_meta::get(pool.get_ref(), "plex_machine_id")
                .await
                .ok()
                .flatten();
            match (rating_key, machine_id, cfg.plex_web_url.as_ref()) {
                (Some(rk), Some(mid), Some(base)) => plex_web_url(base, &mid, &rk),
                _ => return HttpResponse::NotFound().finish(),
            }
        }
        Service::Crunchyroll | Service::Disney => {
            let link = sqlx::query_scalar::<_, Option<String>>(
                "SELECT link FROM title_services WHERE title_id = ? AND service = ?",
            )
            .bind(id)
            .bind(service.as_str())
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten()
            .flatten();
            match link {
                Some(u) if is_allowed_motn_link(service, &u) => u,
                Some(_) => {
                    tracing::warn!(
                        "refusing non-allowlisted watch link for {}",
                        service.as_str()
                    );
                    return HttpResponse::NotFound().finish();
                }
                None => return HttpResponse::NotFound().finish(),
            }
        }
    };

    HttpResponse::Found()
        .insert_header(("Location", location))
        .finish()
}

// ── tests ────────────────────────────────────────────────────────────────────

/// Integration tests for the redirect handler.
/// NOTE: this module imports `actix_web::test`, which shadows the built-in
/// `#[test]` attribute. Pure sync tests live in `guard_tests` below.
#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use sqlx::SqlitePool;

    use super::WatchConfig;
    use crate::db::{app_meta, init_pool};
    use crate::routes;

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        (init_pool(&url).await.unwrap(), dir)
    }

    #[actix_web::test]
    async fn plex_redirects_when_resolvable() {
        let (p, _dir) = pool().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type, plex_rating_key) VALUES ('T', 2020, 'movie', '49518') RETURNING id",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        app_meta::set(&p, "plex_machine_id", "MID").await.unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(p))
                .app_data(web::Data::new(WatchConfig {
                    plex_web_url: Some("http://lan:32400".into()),
                }))
                .configure(routes::configure),
        )
        .await;

        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/watch/plex"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 302);
        let loc = resp.headers().get("Location").unwrap().to_str().unwrap();
        assert!(loc.contains("/server/MID/details"));
        assert!(loc.contains("49518"));
    }

    #[actix_web::test]
    async fn crunchyroll_redirects_to_link() {
        let (p, _dir) = pool().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type) VALUES ('A', 2021, 'series') RETURNING id",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        sqlx::query("INSERT INTO title_services (title_id, service, link) VALUES (?, 'crunchyroll', 'https://www.crunchyroll.com/series/x')")
            .bind(id)
            .execute(&p)
            .await
            .unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(p))
                .app_data(web::Data::new(WatchConfig { plex_web_url: None }))
                .configure(routes::configure),
        )
        .await;

        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/watch/crunchyroll"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 302);
        assert_eq!(
            resp.headers().get("Location").unwrap().to_str().unwrap(),
            "https://www.crunchyroll.com/series/x"
        );
    }

    #[actix_web::test]
    async fn missing_link_is_404_and_bad_service_is_400() {
        let (p, _dir) = pool().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type) VALUES ('B', 2021, 'movie') RETURNING id",
        )
        .fetch_one(&p)
        .await
        .unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(p))
                .app_data(web::Data::new(WatchConfig { plex_web_url: None }))
                .configure(routes::configure),
        )
        .await;

        let r404 = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/api/titles/{id}/watch/plex"))
                .to_request(),
        )
        .await;
        assert_eq!(r404.status(), 404);

        let r400 = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/api/titles/{id}/watch/netflix"))
                .to_request(),
        )
        .await;
        assert_eq!(r400.status(), 400);
    }
}

/// Pure-function unit tests — kept separate from `tests` so that the
/// `actix_web::test` import there does not shadow the built-in `#[test]`
/// attribute and break sync fn compilation (matches the pattern in images.rs).
#[cfg(test)]
mod guard_tests {
    use super::{is_allowed_motn_link, plex_web_url};
    use crate::models::Service;

    #[test]
    fn builds_plex_web_url() {
        assert_eq!(
            plex_web_url("http://lan:32400/", "MID", "49518"),
            "http://lan:32400/web/index.html#!/server/MID/details?key=%2Flibrary%2Fmetadata%2F49518"
        );
    }

    #[test]
    fn motn_link_allowlist() {
        assert!(is_allowed_motn_link(
            Service::Crunchyroll,
            "https://www.crunchyroll.com/x"
        ));
        assert!(!is_allowed_motn_link(
            Service::Crunchyroll,
            "http://www.crunchyroll.com/x"
        ));
        assert!(!is_allowed_motn_link(
            Service::Crunchyroll,
            "https://evil.example/x"
        ));
        assert!(!is_allowed_motn_link(
            Service::Plex,
            "https://crunchyroll.com/x"
        ));
    }
}
