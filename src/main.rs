pub mod template;

use std::{
    borrow::Cow,
    collections::HashMap,
    net::{IpAddr, Ipv6Addr},
    path::{Path, PathBuf},
};

use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    response::{Html, Redirect},
    routing::{get, post},
    Router,
};
use pnet::ipnetwork::Ipv6Network;
use qrcode::QrCode;
use tokio::{fs::File, io::AsyncWriteExt};
use tower_http::services::ServeDir;

use crate::template::Template;

const PORT: u16 = 8000;

#[derive(Debug, Clone)]
struct AppState {
    file_dir: PathBuf,
    listen_urls: Vec<ListenUrl>,
}

#[derive(Debug, Clone)]
struct ListenUrl {
    is_loopback: bool,
    interface: String,
    ip: IpAddr,
    url: String,
    qr_code_svg: String,
}

#[derive(Debug, Clone)]
struct Templates {
    root: Template,
    file_list_item: Template,
    qr_code_item: Template,
    connection_item: Template,
}

#[derive(Debug, Clone)]
struct FileInfo {
    name: String,
    size: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let file_dir = get_file_directory();
    if !file_dir.exists() {
        std::fs::create_dir_all(file_dir.clone()).unwrap();
    }
    let file_dir = file_dir.canonicalize().unwrap();
    println!("Storing files in {}", file_dir.to_string_lossy());

    let listen_urls = list_urls();

    let app_state = AppState {
        file_dir,
        listen_urls,
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

    let listener = tokio::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, PORT))
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}

fn get_file_directory() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("filedrop");
    dir
}

fn list_urls() -> Vec<ListenUrl> {
    let ipv6_link_local: Ipv6Network = "fe80::/10".parse().unwrap();

    let mut result = pnet::datalink::interfaces()
        .into_iter()
        .filter(|interface| interface.is_lower_up())
        .flat_map(|interface| {
            let is_loopback = interface.is_loopback();
            interface
                .ips
                .into_iter()
                .filter(|ip| match ip.ip() {
                    IpAddr::V4(_) => true,
                    IpAddr::V6(ipv6) => !ipv6_link_local.contains(ipv6),
                })
                .map(move |ip| {
                    let ip = ip.ip();
                    let url = if ip.is_ipv4() {
                        format!("http://{ip}:{PORT}")
                    } else {
                        format!("http://[{ip}]:{PORT}")
                    };
                    let interface = interface.name.clone();
                    let qr_code_svg = render_url_svg(&url);

                    ListenUrl {
                        is_loopback,
                        interface,
                        ip,
                        url,
                        qr_code_svg,
                    }
                })
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|ip| (!ip.is_loopback, ip.interface.clone(), ip.ip));
    result
}

fn render_url_svg(url: &str) -> String {
    let xml = QrCode::new(url)
        .unwrap()
        .render()
        .min_dimensions(200, 200)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .quiet_zone(false)
        .build();
    xml.split_once("?>").unwrap().1.to_string()
}

async fn list_files(file_dir: &Path) -> Vec<FileInfo> {
    let mut results = vec![];
    let mut reader = tokio::fs::read_dir(file_dir).await.unwrap();
    while let Some(dir_entry) = reader.next_entry().await.unwrap() {
        let meta = dir_entry.metadata().await.unwrap();
        if meta.is_file() {
            let name = dir_entry.file_name().to_string_lossy().to_string();
            let size = format_size(meta.len());
            let file_info = FileInfo { name, size };
            results.push(file_info);
        }
    }
    results.sort_by_key(|r| r.name.clone());
    results
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KiB", (bytes as f32) / 1024.0)
    } else {
        format!("{:.2} MiB", (bytes as f32) / 1024.0 / 1024.0)
    }
}

async fn list_files_html(State(app_state): State<AppState>) -> Html<String> {
    let files = list_files(&app_state.file_dir).await;

    let templates = {
        let templates = if cfg!(debug_assertions) {
            Cow::Owned(tokio::fs::read_to_string("src/index.html").await.unwrap())
        } else {
            Cow::Borrowed(include_str!("index.html"))
        };
        let templates = Template::many(&templates);

        let [root, file_list_item, qr_code_item, connection_item] = templates.as_slice() else {
            unreachable!()
        };
        Templates {
            root: root.clone(),
            file_list_item: file_list_item.clone(),
            connection_item: connection_item.clone(),
            qr_code_item: qr_code_item.clone(),
        }
    };

    let file_listing = templates
        .file_list_item
        .render_many(files.iter().map(|file| {
            HashMap::from([
                ("file_name".to_string(), file.name.as_str()),
                ("size".to_string(), file.size.as_str()),
            ])
        }));

    let qr_code_listing = templates.qr_code_item.render_many(
        app_state
            .listen_urls
            .iter()
            .filter(|url| !url.is_loopback)
            .map(|url| HashMap::from([("svg".to_string(), url.qr_code_svg.as_str())])),
    );

    let connection_listing = templates.connection_item.render_many(
        app_state
            .listen_urls
            .iter()
            .filter(|url| !url.is_loopback)
            .map(|url| {
                HashMap::from([
                    ("interface".to_string(), url.interface.clone()),
                    ("ip".to_string(), url.ip.to_string()),
                ])
            }),
    );

    let html = templates.root.render(&HashMap::from([
        ("file_listing".to_string(), file_listing.as_str()),
        ("qr_code_listing".to_string(), qr_code_listing.as_str()),
        (
            "connection_listing".to_string(),
            connection_listing.as_str(),
        ),
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
