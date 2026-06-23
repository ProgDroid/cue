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
    const fn columns(self) -> (&'static str, &'static str) {
        match self {
            Self::Poster => ("poster_url", "poster_plex"),
            Self::Backdrop => ("backdrop_url", "backdrop_plex"),
        }
    }
}

const CACHE: (&str, &str) = ("Cache-Control", "public, max-age=86400");

async fn serve(id: i64, kind: Kind, pool: &SqlitePool, art: &PlexArt) -> HttpResponse {
    let (url_col, plex_col) = kind.columns();
    let sql = format!("SELECT {url_col}, {plex_col} FROM titles WHERE id = ?");
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await;
    let Ok(Some((url, plex_path))) = row else {
        return HttpResponse::NotFound().finish();
    };

    if let Some(u) = url {
        return HttpResponse::Found()
            .insert_header(("Location", u))
            .insert_header(CACHE)
            .finish();
    }

    let Some(path) = plex_path else {
        return HttpResponse::NotFound().finish();
    };
    let (Some(base), Some(token)) = (art.base_url.as_ref(), art.token.as_ref()) else {
        return HttpResponse::NotFound().finish();
    };
    let upstream = format!(
        "{}{}?X-Plex-Token={}",
        base.trim_end_matches('/'),
        path,
        token
    );
    match reqwest::get(&upstream).await {
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
        let (pool, _dir, id) = pool_with_title(Some("https://cdn/p.jpg"), None).await;
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
            "https://cdn/p.jpg"
        );
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
