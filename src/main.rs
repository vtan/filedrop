pub mod template;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

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

use crate::template::Template;

const PORT: u16 = 8000;

#[derive(Debug, Clone)]
struct AppState {
    file_dir: PathBuf,
    listen_urls: Vec<ListenUrl>,
    templates: Templates,
}

#[derive(Debug, Clone)]
struct ListenUrl {
    url: String,
    qr_code_svg: String,
}

#[derive(Debug, Clone)]
struct Templates {
    root: Template,
    file_list_item: Template,
    qr_code_item: Template,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let templates = {
        let templates = Template::many(include_str!("index.html"));
        let [root, file_list_item, qr_code_item] = templates.as_slice() else {
            unreachable!()
        };
        Templates {
            root: root.clone(),
            file_list_item: file_list_item.clone(),
            qr_code_item: qr_code_item.clone(),
        }
    };

    let file_dir = get_file_directory();
    if !file_dir.exists() {
        std::fs::create_dir_all(file_dir.clone()).unwrap();
    }
    let file_dir = file_dir.canonicalize().unwrap();
    println!("Storing files in {}", file_dir.to_string_lossy());

    let listen_urls = {
        let urls = list_urls();
        urls.into_iter()
            .map(|url| {
                let qr_code_svg = render_url_svg(&url);
                ListenUrl { url, qr_code_svg }
            })
            .collect()
    };

    let app_state = AppState {
        file_dir,
        listen_urls,
        templates,
    };

    for url in &app_state.listen_urls {
        println!("Listening at {}", url.url);
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

fn render_url_svg(url: &str) -> String {
    let xml = QrCode::new(url)
        .unwrap()
        .render()
        .min_dimensions(200, 200)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    xml.split_once("?>").unwrap().1.to_string()
}

async fn list_files(file_dir: &Path) -> Vec<DirEntry> {
    let mut entries = vec![];
    let mut reader = tokio::fs::read_dir(file_dir).await.unwrap();
    while let Some(dir_entry) = reader.next_entry().await.unwrap() {
        let meta = dir_entry.metadata().await.unwrap();
        if meta.is_file() {
            entries.push(dir_entry);
        }
    }
    entries.sort_by_key(|e| e.file_name());
    entries
}

async fn list_files_html(State(app_state): State<AppState>) -> Html<String> {
    let files = list_files(&app_state.file_dir).await;

    let file_listing = app_state
        .templates
        .file_list_item
        .render_many(files.iter().map(|file| {
            HashMap::from([(
                "file_name".to_string(),
                file.file_name().to_string_lossy().to_string(),
            )])
        }));

    let qr_code_listing =
        app_state
            .templates
            .qr_code_item
            .render_many(app_state.listen_urls.iter().map(|url| {
                HashMap::from([
                    ("url".to_string(), url.url.as_str()),
                    ("svg".to_string(), &url.qr_code_svg.as_str()),
                ])
            }));

    let html = app_state.templates.root.render(&HashMap::from([
        ("file_listing".to_string(), file_listing.as_str()),
        ("qr_code_listing".to_string(), qr_code_listing.as_str()),
    ]));

    Html(html)
}

async fn upload_file(State(app_state): State<AppState>, mut multipart: Multipart) -> Redirect {
    while let Some(mut field) = multipart.next_field().await.unwrap() {
        if field.name() == Some("file") {
            if let Some(file_name) = field.file_name().filter(|n| !n.is_empty()) {
                let mut file_path = app_state.file_dir.clone();
                file_path.push(file_name);
                let mut file = File::create(file_path.clone()).await.unwrap();
                while let Some(chunk) = field.chunk().await.unwrap() {
                    file.write_all(&chunk).await.unwrap();
                }

                println!("Uploaded {}", file_path.to_string_lossy());
            }
        }
    }
    Redirect::to("/")
}
