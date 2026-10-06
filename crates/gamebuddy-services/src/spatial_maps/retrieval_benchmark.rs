//! Offline candidate experiments share the production crop and geometry gates.
//! They never publish an observation or alter a player's localization/progress.
use super::*;
use std::{path::Path, sync::Arc};

pub struct Probe {
    index: Arc<Index>,
    maps: Vec<gamebuddy_core::MapAsset>,
    engine: Engine,
    world: World,
    query: Option<Features>,
}

impl Probe {
    pub fn open(store: &Store, guide: &Guide) -> Result<Self> {
        let index = store
            .get_cached::<Index>(&format!("spatial-maps:{}", guide.id))?
            .context("Prepare atlas pixels first")?;
        ensure!(index.complete, "Map index is incomplete");
        let world = crate::spatial::load(store, guide)?.context("Prepare the world first")?;
        ensure!(
            world.profile.dimension == Dimension::TwoD,
            "Atlas benchmark requires 2D maps"
        );
        let engine = Engine::learned(store, Dimension::TwoD, "map-benchmark")?;
        ensure!(
            index.encoder == engine.encoder,
            "Map model changed; rebuild its index"
        );
        let maps = navigation::atlas(store, guide)?.maps;
        Ok(Self {
            index,
            maps,
            engine,
            world,
            query: None,
        })
    }

    pub fn export(&self, store: &Store) -> Value {
        json!({
            "encoder": self.index.encoder,
            "reference_paths": self.world.views.iter().map(|view| &view.image_path).collect::<Vec<_>>(),
            "tiles": self.index.tiles.iter().enumerate().filter(|(_, tile)| self.enabled(tile))
                .map(|(id, tile)| json!({"id":id,"map_id":tile.map_id,
                    "rect":[tile.x,tile.y,tile.width,tile.height],
                    "map_size":[tile.map_width,tile.map_height],
                    "learned_key":tile.features.learned_key})).collect::<Vec<_>>(),
            "maps":self.maps.iter().filter(|map| map.enabled).map(|map|
                json!({"id":map.id,"title":map.title,"path":navigation::cached_map_path(store,map)}))
                .collect::<Vec<_>>()
        })
    }

    fn enabled(&self, tile: &Tile) -> bool {
        self.maps
            .iter()
            .any(|map| map.id == tile.map_id && map.enabled)
    }

    pub fn query(&mut self, image: &image::DynamicImage, background: &Path) -> Result<Value> {
        // Clear the previous query even when extraction of this frame fails.
        self.query = None;
        let query = extract_query(&mut self.engine, &self.world, image, background)?;
        let mut ranked = self
            .index
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, tile)| self.enabled(tile))
            .map(|(id, tile)| (id, spatial::retrieval(&query, &tile.features)))
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        self.query = Some(query);
        Ok(json!({"ranking":ranked,"background":background}))
    }

    pub fn verify(&mut self, ids: &[usize]) -> Result<Option<Value>> {
        ensure!(
            !ids.is_empty() && ids.len() <= 48,
            "Provide 1..48 candidates"
        );
        let mut seen = HashSet::new();
        let mut ranked = Vec::new();
        for &id in ids {
            ensure!(seen.insert(id), "Duplicate candidate");
            let tile = self.index.tiles.get(id).context("Unknown tile")?;
            ensure!(self.enabled(tile), "Disabled map");
            ranked.push((tile, 0.0));
        }
        register_candidates(
            &mut self.engine,
            self.query.as_ref().context("Extract a query first")?,
            &self.maps,
            &ranked,
        )
    }
}
