use actix_files::NamedFile;
use actix_web::{web, HttpRequest, HttpResponse, Responder};

#[derive(Clone)]
pub struct StaticDir(pub String);

const PLACEHOLDER: &str = "<!doctype html><meta charset=utf-8><title>cue</title>\
    <h1>cue backend running</h1><p>Frontend not built yet.</p>";

/// Catch-all SPA handler: file → `index.html` fallback → inline placeholder.
///
/// Serves the requested file when it exists under the static dir; otherwise
/// falls back to `index.html` (SPA client routing); otherwise an inline
/// placeholder (frontend not built yet). Rejects `..` traversal.
#[allow(clippy::future_not_send)] // Actix handlers run on a single-threaded runtime.
pub async fn serve_spa(req: HttpRequest, dir: web::Data<StaticDir>) -> impl Responder {
    let base = std::path::Path::new(&dir.0);
    let rel = req.path().trim_start_matches('/');

    // Reject path traversal before touching the filesystem.
    if !rel.is_empty() && !rel.contains("..") {
        if let Ok(file) = NamedFile::open_async(base.join(rel)).await {
            return file.into_response(&req);
        }
    }

    NamedFile::open_async(base.join("index.html"))
        .await
        .map_or_else(
            |_| {
                HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(PLACEHOLDER)
            },
            |file| file.into_response(&req),
        )
}

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};

    use super::{serve_spa, StaticDir};

    #[actix_web::test]
    async fn fallback_serves_placeholder_when_no_build() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(StaticDir("does/not/exist".to_string())))
                .default_service(web::route().to(serve_spa)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/some/client/route")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        assert!(String::from_utf8_lossy(&body).contains("cue backend running"));
    }

    #[actix_web::test]
    async fn serves_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.js"), b"console.log('cue');").unwrap();

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(StaticDir(
                    dir.path().to_string_lossy().into_owned(),
                )))
                .default_service(web::route().to(serve_spa)),
        )
        .await;

        let req = test::TestRequest::get().uri("/app.js").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("console.log('cue');"));
        assert!(!text.contains("Frontend not built yet"));
    }

    #[actix_web::test]
    async fn falls_back_to_index_for_client_route() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            b"<!doctype html><title>cue app shell</title>",
        )
        .unwrap();

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(StaticDir(
                    dir.path().to_string_lossy().into_owned(),
                )))
                .default_service(web::route().to(serve_spa)),
        )
        .await;

        // A client-side route with no matching file serves index.html (SPA
        // routing), not the inline placeholder.
        let req = test::TestRequest::get()
            .uri("/library/detail/7")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("cue app shell"));
        assert!(!text.contains("Frontend not built yet"));
    }

    #[actix_web::test]
    async fn rejects_path_traversal() {
        // The static dir is a subdir; the secret sits one level up, outside it.
        let root = tempfile::tempdir().unwrap();
        let static_dir = root.path().join("static");
        std::fs::create_dir(&static_dir).unwrap();
        std::fs::write(static_dir.join("index.html"), b"app shell").unwrap();
        std::fs::write(root.path().join("secret.txt"), b"TOP SECRET").unwrap();

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(StaticDir(
                    static_dir.to_string_lossy().into_owned(),
                )))
                .default_service(web::route().to(serve_spa)),
        )
        .await;

        let req = test::TestRequest::get().uri("/../secret.txt").to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let text = String::from_utf8_lossy(&body);
        assert!(
            !text.contains("TOP SECRET"),
            "path traversal must not escape the static dir"
        );
    }
}
