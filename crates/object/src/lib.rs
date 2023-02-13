use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use glam::{Quat, Vec3};
use kiwi_core::{asset_cache, async_ecs::async_run, hierarchy::children, runtime, transform};
use kiwi_decals::decal;
use kiwi_ecs::{
    components, query, query_mut, uid, Debuggable, Description, DeserWorldWithWarnings, EntityData, EntityId, EntityUid, Name, Networked,
    Store, SystemGroup, World,
};
use kiwi_model::{model_def, ModelDef};
use kiwi_network::client::GameRpcArgs;
use kiwi_physics::collider::collider;
use kiwi_rpc::RpcRegistry;
use kiwi_std::{
    asset_cache::{AssetCache, AsyncAssetKey, AsyncAssetKeyExt, SyncAssetKeyExt},
    asset_url::{AbsAssetUrl, AssetUrl, ServerBaseUrlKey},
    download_asset::{AssetError, BytesFromUrl},
    friendly_id, unwrap_log_err,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

components!("object", {
    @[Debuggable, Networked, Store, Name["Object from url"], Description["Load and attach an object from a url or relative path"]]
    object_from_url: String,
});

pub fn systems() -> SystemGroup {
    SystemGroup::new(
        "object",
        vec![query(object_from_url()).spawned().to_system(|q, world, qs, _| {
            let mut to_load = HashMap::<String, Vec<EntityId>>::new();
            for (id, url) in q.collect_cloned(world, qs) {
                let url = if url.ends_with("/objects/main.json") { url } else { format!("{}/objects/main.json", url) };
                to_load.entry(url).or_default().push(id);
            }
            for (url, ids) in to_load {
                let assets = world.resource(asset_cache()).clone();
                let url = unwrap_log_err!(AssetUrl::parse(url));
                let url = ObjectFromUrl(unwrap_log_err!(url.resolve(&ServerBaseUrlKey.get(&assets))));
                let runtime = world.resource(runtime()).clone();
                let async_run = world.resource(async_run()).clone();
                runtime.spawn(async move {
                    let obj = unwrap_log_err!(url.get(&assets).await);
                    let base_ent_id = obj.resource(children())[0];
                    // TODO: This only handles objects with a single entity
                    let entity = obj.clone_entity(base_ent_id).unwrap();
                    async_run.run(move |world| {
                        for id in ids {
                            world.add_components(id, entity.clone());
                        }
                    });
                });
            }
        })],
    )
}

/// EntityUid for a collection of entities. Use `get_uid` to get indiviual entities uids
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MultiEntityUID(EntityUid);
impl MultiEntityUID {
    pub fn new() -> Self {
        Self(EntityUid(friendly_id()))
    }
    pub fn get_uid(&self, index: usize) -> EntityUid {
        EntityUid(format!("{}_{}", self.0, index))
    }
}

pub struct SpawnConfig {
    /// The first object will be named {entity_uid_base}_0 and so on
    pub entity_uids: MultiEntityUID,
    pub components: EntityData,
}

impl SpawnConfig {
    pub fn new(entity_uids: MultiEntityUID, position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        assert!(scale.is_finite());
        assert!(rotation.is_finite());
        assert!(position.is_finite());
        Self {
            entity_uids,
            components: EntityData::new()
                .set(transform::translation(), position)
                .set(transform::rotation(), rotation)
                .set(transform::scale(), scale),
        }
    }
}

fn resolve_object_url(world: &World, url: &str) -> anyhow::Result<ObjectFromUrl> {
    Ok(ObjectFromUrl(AssetUrl::parse(url)?.resolve(&ServerBaseUrlKey.get(world.resource(asset_cache())))?))
}

/// This method assumes the object has already been loaded into the asset cache;
/// Use Object2FromUrl(url).get(&assets).await or rpc_load_object(url)
pub fn spawn_preloaded_by_url(world: &mut World, object_url: String, config: SpawnConfig) -> anyhow::Result<Vec<EntityId>> {
    if let Some(Ok(es)) = resolve_object_url(world, &object_url)?.peek(world.resource(asset_cache())) {
        Ok(spawn(world, &es, config))
    } else {
        Err(anyhow::anyhow!("Object url {} has not been pre-loaded", object_url))
    }
}
pub async fn spawn_by_url(world: &World, object_url: String, config: SpawnConfig) -> anyhow::Result<Vec<EntityId>> {
    let async_run = world.resource(async_run()).clone();
    let es = resolve_object_url(world, &object_url)?.get(world.resource(asset_cache())).await?;
    let (send, recv) = oneshot::channel();
    async_run.run(move |world| {
        send.send(spawn(world, &es, config)).ok();
    });
    Ok(recv.await?)
}
pub fn fire_spawn_by_url(
    world: &World,
    object_url: String,
    config: SpawnConfig,
    cb: Option<Box<dyn FnOnce(&mut World, anyhow::Result<Vec<EntityId>>) + Sync + Send>>,
) -> anyhow::Result<()> {
    let async_run = world.resource(async_run()).clone();
    let assets = world.resource(asset_cache()).clone();
    let obj_url = resolve_object_url(world, &object_url)?;
    world.resource(runtime()).spawn(async move {
        let obj = obj_url.get(&assets).await;
        match obj {
            Ok(obj) => {
                async_run.run(move |world| {
                    let ids = spawn(world, &obj, config);
                    if let Some(cb) = cb {
                        cb(world, Ok(ids))
                    }
                });
            }
            Err(err) => {
                if let Some(cb) = cb {
                    async_run.run(move |world| cb(world, Err(err.into())));
                } else {
                    log::error!("Failed to load object: {:?}", err)
                }
            }
        }
    });
    Ok(())
}
fn spawn(world: &mut World, object: &World, config: SpawnConfig) -> Vec<EntityId> {
    let ids = object.spawn_into_world(world, Some(config.components));
    for (i, id) in ids.iter().enumerate() {
        world.add_component(*id, uid(), config.entity_uids.get_uid(i)).unwrap();
    }

    ids
}

pub fn register_rpcs(reg: &mut RpcRegistry<GameRpcArgs>) {
    reg.register(rpc_load_object);
}

pub async fn rpc_load_object(args: GameRpcArgs, url: String) {
    let assets = {
        let state = args.state.lock();
        state.get_player_world(&args.user_id).map(|world| world.resource(asset_cache()).clone())
    };
    if let Some(assets) = assets {
        if let Err(err) = ObjectFromUrl(unwrap_log_err!(AbsAssetUrl::parse(url))).get(&assets).await {
            log::error!("Failed to load object: {:?}", err);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObjectFromUrl(pub AbsAssetUrl);
#[async_trait]
impl AsyncAssetKey<Result<Arc<World>, AssetError>> for ObjectFromUrl {
    async fn load(self, assets: AssetCache) -> Result<Arc<World>, AssetError> {
        let data = BytesFromUrl::new(self.0.clone(), true).get(&assets).await?;
        let DeserWorldWithWarnings { mut world, warnings } = tokio::task::block_in_place(|| serde_json::from_slice(&data))
            .with_context(|| format!("Failed to deserialize object2 from url {}", self.0))?;
        warnings.log_warnings();
        for (_id, (url,), _) in query_mut((model_def(),), ()).iter(&mut world, None) {
            *url = ModelDef(url.0.resolve(&self.0).context("Failed to resolve model url")?.into());
        }
        for (_id, (def,), _) in query_mut((collider(),), ()).iter(&mut world, None) {
            def.resolve(&self.0).context("Failed to resolve collider")?;
        }
        for (_id, (def,), _) in query_mut((decal(),), ()).iter(&mut world, None) {
            *def = def.resolve(&self.0).context("Failed to resolve decal")?.into();
        }
        Ok(Arc::new(world))
    }
}
