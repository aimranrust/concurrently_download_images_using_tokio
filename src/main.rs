use std::{collections::HashMap, fs::File, io::Write, path::Path};
use serde::Deserialize;
use tokio::fs;
use futures::stream::{FuturesUnordered, StreamExt};

#[derive(Debug, Deserialize)]
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
    // Load JSON file (array of objects)
    let data = fs::read_to_string("./src/products.json").await?;
    let products: Vec<Product> = serde_json::from_str(&data)?;
if let Some(first) = products.first() {
    println!("First product: {:#?}", first);
}



    let client = reqwest::Client::new();

    // Use FuturesUnordered for concurrent downloads
    let mut tasks = FuturesUnordered::new();

    for product in products {
        if let Some(url) = product.image_url {
            let filename = if product.localfilename.is_empty() {
                format!("{}.jpg", product.dbhash)
            } else {
                product.localfilename.clone()
            };

            let client = client.clone();

            tasks.push(tokio::spawn(async move {
                match maybe_download(&client, &url, &filename).await {
                    Ok(true) => println!("✅ Downloaded {url} → {filename}"),
                    Ok(false) => println!("⏩ Skipped {filename}, already exists"),
                    Err(e) => eprintln!("❌ Failed {url}: {e}"),
                }
            }));
        }
    }

    // Await all download tasks
    while let Some(_) = tasks.next().await {}

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
