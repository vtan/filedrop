#![recursion_limit = "512"]

use std::path::{Path, PathBuf};

use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    response::{Html, Redirect},
    routing::{get, post},
    Router,
};
use qrcode::QrCode;
use tokio::{
    fs::{DirEntry, File},
    io::AsyncWriteExt,
};
use tower_http::services::ServeDir;

const PORT: u16 = 8000;

#[derive(Debug, Clone)]
struct AppState {
    file_dir: PathBuf,
    urls: Vec<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let file_dir = get_file_directory();
    if !file_dir.exists() {
        std::fs::create_dir_all(file_dir.clone()).unwrap();
    }
    let file_dir = file_dir.canonicalize().unwrap();
    println!("Storing files in {}", file_dir.to_string_lossy());

    let urls = list_urls();

    let app_state = AppState { file_dir, urls };

    for url in &app_state.urls {
        println!("Listening at {url}");
    }

    let serve_dir = ServeDir::new(app_state.file_dir.clone());

    let app = Router::new()
        .route("/", get(list_files_html))
        .route("/upload", post(upload_file))
        .nest_service("/files", serve_dir)
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", PORT))
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}

fn get_file_directory() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("filedrop");
    dir
}

fn list_urls() -> Vec<String> {
    pnet::datalink::interfaces()
        .into_iter()
        .filter(|interface| {
            interface.is_up() && interface.is_lower_up() && !interface.is_loopback()
        })
        .flat_map(|interface| interface.ips.into_iter())
        .filter(|ip| ip.is_ipv4())
        .map(|ip| {
            let ip = ip.ip();
            format!("http://{ip}:{PORT}")
        })
        .collect()
}

async fn list_files(file_dir: &Path) -> Vec<DirEntry> {
    let mut entries = vec![];
    let mut reader = tokio::fs::read_dir(file_dir).await.unwrap();
    while let Some(dir_entry) = reader.next_entry().await.unwrap() {
        entries.push(dir_entry);
    }
    entries.sort_by_key(|e| e.file_name());
    entries
}

async fn list_files_html(State(app_state): State<AppState>) -> Html<String> {
    let files = list_files(&app_state.file_dir).await;

    let html = html::root::Html::builder()
        .lang("en")
        .head(|head| {
            head.meta(|meta| meta.charset("utf-8"))
                .meta(|meta| {
                    meta.name("viewport")
                        .content("width=device-width, initial-scale=1")
                })
                .title(|title| title.text("filedrop"))
        })
        .body(|body| {
            body.heading_1(|h| h.text("Upload a file"))
                .form(|form| {
                    form.action("/upload")
                        .method("post")
                        .enctype("multipart/form-data")
                        .division(|div| {
                            div.input(|input| input.type_("file").name("file").required(""))
                        })
                        .button(|button| button.text("Upload"))
                })
                .heading_1(|h| h.text("Uploaded files"))
                .unordered_list(|mut ul| {
                    for dir_entry in &files {
                        let file_name = dir_entry.file_name().to_string_lossy().to_string();
                        let url = format!("/files/{file_name}");
                        ul = ul.list_item(|li| li.anchor(|a| a.href(url).text(file_name)));
                    }
                    if files.is_empty() {
                        ul = ul.list_item(|li| li.text("No files"))
                    }
                    ul
                })
                .heading_1(|h| h.text("Connection"))
                .division(|mut div| {
                    for url in &app_state.urls {
                        let qr_code = QrCode::new(url)
                            .unwrap()
                            .render::<qrcode::render::unicode::Dense1x2>()
                            .dark_color(qrcode::render::unicode::Dense1x2::Light)
                            .light_color(qrcode::render::unicode::Dense1x2::Dark)
                            .build();
                        div = div
                            .preformatted_text(|pre| pre.style("font-size: 10px").text(qr_code))
                            .span(|span| span.text(url.clone()));
                    }
                    div
                })
        })
        .build()
        .to_string();

    Html(html)
}

async fn upload_file(State(app_state): State<AppState>, mut multipart: Multipart) -> Redirect {
    while let Some(mut field) = multipart.next_field().await.unwrap() {
        if field.name() == Some("file") {
            let mut file_name = app_state.file_dir.clone();
            file_name.push(field.file_name().unwrap());
            let mut file = File::create(file_name.clone()).await.unwrap();
            while let Some(chunk) = field.chunk().await.unwrap() {
                file.write_all(&chunk).await.unwrap();
            }

            println!("Uploaded {}", file_name.to_string_lossy());
            return Redirect::to("/");
        }
    }
    panic!("No file field");
}
