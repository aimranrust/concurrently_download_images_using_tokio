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

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Product {
    #[serde(rename = "$iFrame_IMAGE")]
    image_url: Option<String>,

    #[serde(default)]
    localfilename: String,

    #[serde(default)]
    dbhash: String,

    // Capture all other fields dynamically
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --------- find folder of the running exe -------------
    let exe_dir: PathBuf = env::current_exe()?.parent().unwrap().to_path_buf();

    // --------- read JSON file name from command line ------
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <json-file>", args[0]);
        std::process::exit(1);
    }
    let json_file_name = &args[1];
    let json_path = exe_dir.join(json_file_name);

    // --------- load products ------------------------------
    let data = fs::read_to_string(&json_path).await?;
    let mut products: Vec<Product> = serde_json::from_str(&data)?;

    // --------- update each product's image_url ------------
    const SERVER_IMAGES_FOLDER: &str = "images";
    for p in &mut products {
        if let Some(_) = p.image_url {
            let brandfolder = p
                .extra
                .get("$iBrand")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .replace(' ', "_")
                .replace('\'', "")
                .to_lowercase();
            p.image_url = Some(format!(
                "https://portal.framescloud.optiserver.co.uk/{}/{}/{}",
                SERVER_IMAGES_FOLDER, brandfolder, p.localfilename
            ));
        }
    }

    let client = reqwest::Client::new();

    let total = products.len();
    let counter = Arc::new(AtomicUsize::new(0));

    // --------- concurrent downloads -----------------------
    let mut tasks = FuturesUnordered::new();

    for product in products.clone() {
        if let Some(url) = product.image_url.clone() {
            let filename = if product.localfilename.is_empty() {
                format!("{}.jpg", product.dbhash)
            } else {
                product.localfilename.clone()
            };

            // create brand subfolder if necessary
            if let Some(brand) = product
                .extra
                .get("$iBrand")
                .and_then(|v| v.as_str())
            {
                let brandfolder = brand
                    .replace(' ', "_")
                    .replace('\'', "")
                    .to_lowercase();
                tokio::fs::create_dir_all(&brandfolder).await?;
                let filename = format!("{}/{}", brandfolder, filename);

                let client = client.clone();
                let counter = counter.clone();

                tasks.push(tokio::spawn(async move {
                    match maybe_download(&client, &url, &filename).await {
                        Ok(true) => {
                            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                            println!("✅ Downloading {n} of {total}: {url} → {filename}");
                        }
                        Ok(false) => {
                            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                            println!("⏩ Skipped {n} of {total}: {filename}, already exists");
                        }
                        Err(e) => eprintln!("❌ Failed {url}: {e}"),
                    }
                }));
            }
        }
    }

    // wait for all downloads to complete
    while let Some(_) = tasks.next().await {}

    // --------- save modified JSON -------------------------
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

async fn maybe_download(
    client: &reqwest::Client,
    url: &str,
    filename: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    // If file already exists, skip
    if fs::try_exists(filename).await? {
        return Ok(false);
    }

    // Otherwise download
    let resp = client.get(url).send().await?;
    let bytes = resp.bytes().await?;

    let mut file = File::create(Path::new(filename))?;
    file.write_all(&bytes)?;

    Ok(true)
}
