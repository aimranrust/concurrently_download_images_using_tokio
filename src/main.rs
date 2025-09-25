use std::{
    collections::HashMap,
    env,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::fs;

/// Product as stored in JSON.
///
/// `$iFrame_IMAGE` in the input is read into `original_image_url` and is
/// used for downloading.  
/// `$iFrame_IMAGE` in the **output** is written from `new_image_url`.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct Product {
    /// The URL from the input JSON we will download from
    #[serde(rename = "$iFrame_IMAGE")]
    original_image_url: Option<String>,

    #[serde(default)]
    localfilename: String,

    #[serde(default)]
    dbhash: String,

    /// The URL to write back to the output JSON
    #[serde(rename = "$iFrame_IMAGE")]
    #[serde(skip_deserializing)]
    new_image_url: Option<String>,

    /// Any other fields we don't explicitly model
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Location of the executable
    let exe_dir: PathBuf = env::current_exe()?.parent().unwrap().to_path_buf();

    // JSON file passed as the first command-line argument
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <json-file>", args[0]);
        std::process::exit(1);
    }
    let json_file_name = &args[1];
    let json_path = exe_dir.join(json_file_name);

    // Load the JSON file
    let data = fs::read_to_string(&json_path).await?;
    let mut products: Vec<Product> = serde_json::from_str(&data)?;

    // Prepare the new URLs that will be written to the modified JSON
    const SERVER_IMAGES_FOLDER: &str = "images";
    for p in &mut products {
        if p.original_image_url.is_some() {
            let brandfolder = p
                .extra
                .get("$iBrand")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .replace(' ', "_")
                .replace('\'', "")
                .to_lowercase();
            p.new_image_url = Some(format!(
                "https://portal.framescloud.optiserver.co.uk/{}/{}/{}",
                SERVER_IMAGES_FOLDER,
                brandfolder,
                p.localfilename
            ));
        }
    }

    let client = reqwest::Client::new();
    let total = products.len();
    let counter = Arc::new(AtomicUsize::new(0));
    let mut tasks = FuturesUnordered::new();

    // Download each image concurrently
    for product in products.clone() {
        if let Some(url) = product.original_image_url.clone() {
            let filename = if product.localfilename.is_empty() {
                format!("{}.jpg", product.dbhash)
            } else {
                product.localfilename.clone()
            };

            if let Some(brand) = product.extra.get("$iBrand").and_then(|v| v.as_str()) {
                let brandfolder = brand
                    .replace(' ', "_")
                    .replace('\'', "")
                    .to_lowercase();
                tokio::fs::create_dir_all(&brandfolder).await?;
                let filepath = format!("{}/{}", brandfolder, filename);

                let client = client.clone();
                let counter = counter.clone();
                let total = total;

                tasks.push(tokio::spawn(async move {
                    match maybe_download(&client, &url, &filepath).await {
                        Ok(true) => {
                            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                            println!("✅ Downloaded {n} of {total}: {url} → {filepath}");
                        }
                        Ok(false) => {
                            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                            println!("⏩ Skipped {n} of {total}: {filepath}, already exists");
                        }
                        Err(e) => eprintln!("❌ Failed {url}: {e}"),
                    }
                }));
            }
        }
    }

    // Wait for all downloads to finish
    while let Some(_) = tasks.next().await {}

    // Write the modified JSON with new URLs
    let new_name = format!(
        "{}-modified-w-embedded-imgs.json",
        json_file_name.trim_end_matches(".json")
    );
    let new_path = exe_dir.join(new_name);
    let mut out_file = File::create(&new_path)?;
    let updated_json = serde_json::to_string_pretty(&products)?;
    out_file.write_all(updated_json.as_bytes())?;

    println!("Modified JSON written to {}", new_path.display());

    Ok(())
}

/// Download `url` to `filename` unless it already exists.
/// Returns Ok(true) if a download occurred, Ok(false) if skipped.
async fn maybe_download(
    client: &reqwest::Client,
    url: &str,
    filename: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    if fs::try_exists(filename).await? {
        return Ok(false);
    }

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("HTTP error {}", resp.status()).into());
    }

    let bytes = resp.bytes().await?;
    let mut file = File::create(Path::new(filename))?;
    file.write_all(&bytes)?;
    Ok(true)
}
