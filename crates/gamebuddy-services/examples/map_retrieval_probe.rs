//! JSON-lines bridge for scripts/benchmark-map-retrieval.py. Use a private store.
use anyhow::{Context, Result};
use gamebuddy_services::{Store, spatial_maps::retrieval_benchmark::Probe};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    time::Instant,
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let store = Store::open(args.next().context("private store directory")?)?;
    let guide = store.guide(&args.next().context("guide ID")?)?;
    let started = Instant::now();
    let mut probe = Probe::open(&store, &guide)?;
    println!(
        "{}",
        json!({"ready":true,"startup_ms":started.elapsed().as_secs_f64()*1000.0})
    );
    for line in std::io::stdin().lock().lines() {
        let started = Instant::now();
        let result: Result<Value> = (|| {
            let request: Value = serde_json::from_str(&line?)?;
            match request["op"].as_str() {
                Some("index") => Ok(probe.export(&store)),
                Some("query") => {
                    let image = image::open(request["path"].as_str().context("image path")?)?;
                    let path = PathBuf::from(
                        request["background"]
                            .as_str()
                            .context("background output")?,
                    );
                    probe.query(&image, &path)
                }
                Some("verify") => Ok(serde_json::to_value(probe.verify(
                    &serde_json::from_value::<Vec<usize>>(request["ids"].clone())?,
                )?)?),
                _ => anyhow::bail!("Unknown operation"),
            }
        })();
        let response = match result {
            Ok(value) => {
                json!({"result":value,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0})
            }
            Err(error) => json!({"error":format!("{error:#}")}),
        };
        println!("{response}");
        std::io::stdout().flush()?;
    }
    Ok(())
}
