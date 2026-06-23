//! Artwork proxy: 302-redirect to public CDN URLs (MOTN) or stream Plex images
//! with the token injected server-side. The Plex token never reaches the client.

use actix_web::{web, HttpResponse, Responder};
use sqlx::SqlitePool;

/// Plex base URL + token for the proxy-stream branch. Held in app data so the
/// token stays server-side (never serialized into any response).
#[derive(Clone)]
pub struct PlexArt {
    pub base_url: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Copy)]
enum Kind {
    Poster,
    Backdrop,
}

impl Kind {
    /// The two-column `SELECT` for this art kind. Two fully-static strings (not a
    /// `format!`) so there is no string-built SQL to second-guess.
    const fn select_sql(self) -> &'static str {
        match self {
            Self::Poster => "SELECT poster_url, poster_plex FROM titles WHERE id = ?",
            Self::Backdrop => "SELECT backdrop_url, backdrop_plex FROM titles WHERE id = ?",
        }
    }
}

const CACHE: (&str, &str) = ("Cache-Control", "public, max-age=86400");

/// Shared client for the Plex art proxy: explicit timeout (a hung Plex server
/// must not pin an Actix worker) and redirects disabled (a redirect-following
/// fetch of a server-side URL widens the SSRF surface). Built once.
fn proxy_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("build proxy reqwest client")
    })
}

/// Only emit a `Location` to a stored public URL if it is `https://` on the MOTN
/// CDN. The URL originates from an external sync feed, so an unchecked value
/// would turn this proxy into an open redirect (phishing pivot).
fn is_allowed_redirect(raw: &str) -> bool {
    reqwest::Url::parse(raw).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str()
                .is_some_and(|h| h == "movieofthenight.com" || h.ends_with(".movieofthenight.com"))
    })
}

/// Guard the Plex thumb/art path before concatenating it onto the configured
/// Plex base URL: must be an absolute single-slash path with no traversal or
/// scheme/userinfo smuggling that could redirect the server-side fetch elsewhere.
fn is_safe_plex_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("..")
        && !path.contains("://")
        && !path.contains('@')
}

async fn serve(id: i64, kind: Kind, pool: &SqlitePool, art: &PlexArt) -> HttpResponse {
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(kind.select_sql())
        .bind(id)
        .fetch_optional(pool)
        .await;
    let Ok(Some((url, plex_path))) = row else {
        return HttpResponse::NotFound().finish();
    };

    if let Some(u) = url {
        if !is_allowed_redirect(&u) {
            tracing::warn!("refusing to redirect to non-allowlisted art URL");
            return HttpResponse::NotFound().finish();
        }
        return HttpResponse::Found()
            .insert_header(("Location", u))
            .insert_header(CACHE)
            .finish();
    }

    let Some(path) = plex_path else {
        return HttpResponse::NotFound().finish();
    };
    if !is_safe_plex_path(&path) {
        tracing::warn!("refusing to proxy unsafe Plex art path");
        return HttpResponse::NotFound().finish();
    }
    let (Some(base), Some(token)) = (art.base_url.as_ref(), art.token.as_ref()) else {
        return HttpResponse::NotFound().finish();
    };
    let upstream = format!(
        "{}{}?X-Plex-Token={}",
        base.trim_end_matches('/'),
        path,
        token
    );
    match proxy_client().get(&upstream).send().await {
        Ok(resp) if resp.status().is_success() => {
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_owned();
            resp.bytes().await.map_or_else(
                |_| HttpResponse::BadGateway().finish(),
                |bytes| {
                    HttpResponse::Ok()
                        .insert_header(CACHE)
                        .content_type(content_type)
                        .body(bytes)
                },
            )
        }
        Ok(resp) => {
            tracing::warn!("Plex art upstream returned status {}", resp.status());
            HttpResponse::BadGateway().finish()
        }
        Err(e) => {
            // .without_url() strips the upstream URL — which carries the X-Plex-Token — from the error.
            tracing::warn!("Plex art upstream request failed: {}", e.without_url());
            HttpResponse::BadGateway().finish()
        }
    }
}

pub async fn poster(
    id: web::Path<i64>,
    pool: web::Data<SqlitePool>,
    plex: web::Data<PlexArt>,
) -> impl Responder {
    serve(
        id.into_inner(),
        Kind::Poster,
        pool.get_ref(),
        plex.get_ref(),
    )
    .await
}

pub async fn backdrop(
    id: web::Path<i64>,
    pool: web::Data<SqlitePool>,
    plex: web::Data<PlexArt>,
) -> impl Responder {
    serve(
        id.into_inner(),
        Kind::Backdrop,
        pool.get_ref(),
        plex.get_ref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use sqlx::SqlitePool;

    use crate::db::init_pool;
    use crate::routes::{self, images::PlexArt};

    async fn pool_with_title(
        poster_url: Option<&str>,
        poster_plex: Option<&str>,
    ) -> (SqlitePool, tempfile::TempDir, i64) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let url = format!("sqlite:{}", db.to_string_lossy().replace('\\', "/"));
        let pool = init_pool(&url).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO titles (title, year, type, poster_url, poster_plex) VALUES ('T', 2020, 'movie', ?, ?) RETURNING id",
        )
        .bind(poster_url)
        .bind(poster_plex)
        .fetch_one(&pool)
        .await
        .unwrap();
        (pool, dir, id)
    }

    #[actix_web::test]
    async fn poster_redirects_to_public_url() {
        let (pool, _dir, id) =
            pool_with_title(Some("https://cdn.movieofthenight.com/p.jpg"), None).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .app_data(web::Data::new(PlexArt {
                    base_url: None,
                    token: None,
                }))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/poster"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 302);
        assert_eq!(
            resp.headers().get("location").unwrap().to_str().unwrap(),
            "https://cdn.movieofthenight.com/p.jpg"
        );
    }

    #[actix_web::test]
    async fn poster_rejects_disallowed_redirect_host() {
        // A stored URL pointing off the MOTN CDN must NOT become an open redirect.
        let (pool, _dir, id) = pool_with_title(Some("https://evil.example/p.jpg"), None).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .app_data(web::Data::new(PlexArt {
                    base_url: None,
                    token: None,
                }))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/poster"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 404);
    }

    #[actix_web::test]
    async fn poster_rejects_non_https_redirect() {
        let (pool, _dir, id) =
            pool_with_title(Some("http://cdn.movieofthenight.com/p.jpg"), None).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .app_data(web::Data::new(PlexArt {
                    base_url: None,
                    token: None,
                }))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/poster"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 404);
    }

    #[actix_web::test]
    async fn missing_title_is_404() {
        // DB row absent (unknown id) -> 404
        let (pool, _dir, _id) = pool_with_title(None, None).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .app_data(web::Data::new(PlexArt {
                    base_url: None,
                    token: None,
                }))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::get()
            .uri("/api/titles/999999/poster")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 404);
    }

    #[actix_web::test]
    async fn title_without_art_is_404() {
        // DB row present but both image columns NULL -> 404
        let (pool, _dir, id) = pool_with_title(None, None).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .app_data(web::Data::new(PlexArt {
                    base_url: None,
                    token: None,
                }))
                .configure(routes::configure),
        )
        .await;
        let req = test::TestRequest::get()
            .uri(&format!("/api/titles/{id}/poster"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 404);
    }
}

// Pure-function guards live in their own module: the integration `tests` module
// imports `actix_web::test`, which shadows the built-in `#[test]` attribute and
// makes plain sync `#[test]` fns fail to parse.
#[cfg(test)]
mod guard_tests {
    use super::{is_allowed_redirect, is_safe_plex_path};

    #[test]
    fn allowed_redirect_accepts_motn_cdn_https_only() {
        assert!(is_allowed_redirect("https://cdn.movieofthenight.com/a.jpg"));
        assert!(is_allowed_redirect("https://movieofthenight.com/a.jpg"));
        assert!(!is_allowed_redirect("http://cdn.movieofthenight.com/a.jpg"));
        assert!(!is_allowed_redirect("https://evil.example/a.jpg"));
        // No substring/suffix smuggling.
        assert!(!is_allowed_redirect(
            "https://evilmovieofthenight.com/a.jpg"
        ));
        assert!(!is_allowed_redirect(
            "https://cdn.movieofthenight.com.evil.example/a.jpg"
        ));
        assert!(!is_allowed_redirect("not a url"));
    }

    #[test]
    fn safe_plex_path_rejects_traversal_and_smuggling() {
        assert!(is_safe_plex_path("/library/metadata/1/thumb/123"));
        assert!(!is_safe_plex_path("library/metadata")); // no leading slash
        assert!(!is_safe_plex_path("//evil.example/x")); // protocol-relative
        assert!(!is_safe_plex_path("/a/../../etc/passwd")); // traversal
        assert!(!is_safe_plex_path("/a://evil")); // scheme smuggling
        assert!(!is_safe_plex_path("/a@evil.example")); // userinfo smuggling
    }
}
