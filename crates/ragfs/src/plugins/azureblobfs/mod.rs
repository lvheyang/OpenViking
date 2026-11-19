//! Native Azure Blob mount for flat and hierarchical namespaces.
mod hns;
pub mod store;
use self::store::{valid_path, AzureConfig, BlobStore, Condition, Head};
use crate::core::*;
use async_trait::async_trait;
use azure_storage_blob::models::HttpRange;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};
use tokio::sync::RwLock;

/// Built-in plugin registered as `azureblobfs`.
pub struct AzurePlugin;
impl AzurePlugin {
    fn config(config: &PluginConfig) -> Result<AzureConfig> {
        if config.backups.is_some() {
            return Err(Error::Config(
                "azureblobfs requires a single backend".into(),
            ));
        }
        let Some(ConfigValue::Json(value)) = config.params.get("azure_blob") else {
            return Err(Error::Config("missing azure_blob settings".into()));
        };
        serde_json::from_value(value.clone())
            .map_err(|_| Error::Config("invalid azure_blob settings".into()))
    }
}
#[async_trait]
impl ServicePlugin for AzurePlugin {
    fn name(&self) -> &str {
        "azureblobfs"
    }
    fn readme(&self) -> &str {
        "Native Azure Blob content storage (flat or HNS)"
    }
    fn config_params(&self) -> &[ConfigParameter] {
        &[]
    }
    async fn validate(&self, c: &PluginConfig) -> Result<()> {
        BlobStore::new(&Self::config(c)?)?;
        Ok(())
    }
    async fn initialize(&self, c: PluginConfig) -> Result<Box<dyn FileSystem>> {
        Ok(Box::new(AzureFS::open(&Self::config(&c)?).await?))
    }
}

// The move record lives outside nodes/. A dirty mount blocks access until recovery.
// Single active mount per content prefix is required (including during failover).
/// RAGFS filesystem backed by an Azure Blob container.
pub struct AzureFS {
    store: BlobStore,
    gate: RwLock<()>,
    dirty: AtomicBool,
}
#[derive(Serialize, Deserialize)]
struct Move {
    id: String,
    src: String,
    dst: String,
    replace_tag: Option<String>,
}
const JOURNAL: &str = "operations/move";
const PAGE: &str = "operations/page";

/// Only one small page is in flight. Pin each source version before starting the batch.
#[derive(Serialize, Deserialize)]
struct MovePage {
    id: String,
    items: Vec<MoveItem>,
}
#[derive(Serialize, Deserialize)]
struct MoveItem {
    src: String,
    tag: String,
    published: Option<String>,
}

fn node(path: &str) -> Result<String> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return Ok("nodes".into());
    }
    if !valid_path(p) {
        return Err(Error::invalid_path(path));
    }
    // Azure trims trailing dots in some clients. Escape the escape character first.
    let parts: Vec<_> = p
        .split('/')
        .map(|part| {
            let encoded = part.replace('%', "%25");
            match encoded.strip_suffix('.') {
                Some(base) => format!("{base}%2E"),
                None => encoded,
            }
        })
        .collect();
    Ok(format!("nodes/{}", parts.join("/")))
}
fn mutable_node(path: &str) -> Result<String> {
    let key = node(path)?;
    if key == "nodes" {
        return Err(Error::InvalidOperation("mount root is protected".into()));
    }
    Ok(key)
}
fn info(key: &str, h: &Head) -> FileInfo {
    file_info(key, h.size, h.time, &h.meta)
}
fn file_info(
    key: &str,
    size: u64,
    mod_time: SystemTime,
    meta: &HashMap<String, String>,
) -> FileInfo {
    let dir = meta.get("kind").is_some_and(|t| t == "dir");
    FileInfo {
        name: key
            .rsplit('/')
            .next()
            .unwrap_or("")
            .replace("%2E", ".")
            .replace("%25", "%"),
        size,
        mode: meta
            .get("mode")
            .and_then(|m| m.parse().ok())
            .unwrap_or(if dir { 0o755 } else { 0o644 }),
        mod_time,
        is_dir: dir,
    }
}
fn file(h: &Head, key: &str) -> Result<()> {
    if info(key, h).is_dir {
        Err(Error::IsADirectory(key.into()))
    } else {
        Ok(())
    }
}
fn metadata(mode: u32, kind: &str) -> HashMap<String, String> {
    HashMap::from([
        ("mode".into(), mode.to_string()),
        ("kind".into(), kind.into()),
    ])
}
impl AzureFS {
    /// Finish any interrupted move before exposing the mount.
    pub async fn open(cfg: &AzureConfig) -> Result<Self> {
        let fs = Self {
            store: BlobStore::new(cfg)?,
            gate: RwLock::new(()),
            dirty: AtomicBool::new(false),
        };
        fs.store.verify_namespace().await?;
        if let Some(dfs) = &fs.store.dfs {
            dfs.ensure_directories("nodes").await?;
            return Ok(fs);
        }
        match fs.store.head(JOURNAL).await {
            Ok(h) => {
                let m = serde_json::from_slice(&fs.store.get(JOURNAL, &h.tag, None).await?)?;
                fs.finish_move(&m).await?;
                fs.store.delete(JOURNAL, &h.tag).await?;
            }
            Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        Ok(fs)
    }
    fn ready(&self) -> Result<()> {
        if self.dirty.load(Ordering::Acquire) {
            return Err(Error::WouldBlock(
                "mount requires move recovery on reopen".into(),
            ));
        }
        Ok(())
    }
    async fn parent(&self, key: &str) -> Result<()> {
        let (parent, _) = key
            .rsplit_once('/')
            .ok_or_else(|| Error::invalid_path(key))?;
        if parent != "nodes" && !info(parent, &self.store.head(parent).await?).is_dir {
            return Err(Error::NotADirectory(parent.into()));
        }
        Ok(())
    }
    async fn optional(&self, key: &str) -> Result<Option<Head>> {
        match self.store.head(key).await {
            Ok(h) => Ok(Some(h)),
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn finish_move(&self, m: &Move) -> Result<()> {
        if !m.src.starts_with("nodes/")
            || !m.dst.starts_with("nodes/")
            || !valid_path(&m.src)
            || !valid_path(&m.dst)
            || m.src == m.dst
            || m.dst.starts_with(&format!("{}/", m.src))
        {
            return Err(Error::invalid_path("invalid move journal"));
        }
        loop {
            let (mut page, mut page_tag) = if let Some(h) = self.optional(PAGE).await? {
                let page: MovePage =
                    serde_json::from_slice(&self.store.get(PAGE, &h.tag, None).await?)?;
                if page.id != m.id {
                    return Err(Error::InvalidOperation("move journal ID mismatch".into()));
                }
                (page, h.tag)
            } else {
                let mut keys = self
                    .store
                    .key_batch(&format!("{}/", m.src), Some(64))
                    .await?;
                // The directory root is always moved last.
                if keys.is_empty() {
                    if self.optional(&m.src).await?.is_none() {
                        break;
                    }
                    keys.push(m.src.clone());
                }
                let mut items = Vec::with_capacity(keys.len());
                for src in keys {
                    let h = self.store.head(&src).await?;
                    items.push(MoveItem {
                        src,
                        tag: h.tag.to_string(),
                        published: None,
                    });
                }
                let page = MovePage {
                    id: m.id.clone(),
                    items,
                };
                let tag = self
                    .store
                    .put(
                        PAGE,
                        serde_json::to_vec(&page)?.into(),
                        Condition::New,
                        Default::default(),
                    )
                    .await?;
                (page, tag)
            };
            while let Some(item) = page.items.last() {
                let src = item.src.clone();
                if src != m.src && !src.starts_with(&format!("{}/", m.src)) {
                    return Err(Error::invalid_path("move page outside source tree"));
                }
                let dst = format!("{}{}", m.dst, &src[m.src.len()..]);
                let source = self.optional(&src).await?;
                let dest = self.optional(&dst).await?;
                let conflict =
                    || Error::WouldBlock(format!("blob changed during move: {src} -> {dst}"));
                if source
                    .as_ref()
                    .is_some_and(|h| h.tag.to_string() != item.tag)
                {
                    return Err(conflict());
                }
                if let Some(published) = &item.published {
                    if !dest.is_some_and(|h| h.tag.to_string() == *published) {
                        return Err(conflict());
                    }
                    if let Some(h) = source {
                        self.store.delete(&src, &h.tag).await?;
                    }
                    page.items.pop();
                } else {
                    let h = source.ok_or_else(conflict)?;
                    let tag = if let Some(d) = dest.as_ref().filter(|d| {
                        d.meta.get("moveid") == Some(&m.id)
                            && d.meta.get("sourcetag") == Some(&item.tag)
                    }) {
                        // The conditional upload succeeded before the process saved its response.
                        d.tag.clone()
                    } else {
                        let cond = match dest {
                            None if src != m.src || m.replace_tag.is_none() => Condition::New,
                            Some(d)
                                if src == m.src
                                    && m.replace_tag.as_ref() == Some(&d.tag.to_string()) =>
                            {
                                Condition::Match(d.tag)
                            }
                            _ => return Err(conflict()),
                        };
                        let mut meta = h.meta.clone();
                        meta.insert("moveid".into(), m.id.clone());
                        meta.insert("sourcetag".into(), item.tag.clone());
                        self.store.copy(&src, &h, &dst, cond, meta).await?
                    };
                    page.items.last_mut().expect("active item").published = Some(tag.to_string());
                }
                page_tag = self
                    .store
                    .put(
                        PAGE,
                        serde_json::to_vec(&page)?.into(),
                        Condition::Match(page_tag),
                        Default::default(),
                    )
                    .await?;
            }
            self.store.delete(PAGE, &page_tag).await?;
        }
        Ok(())
    }
    async fn move_node(&self, old: &str, new: &str, replace: bool) -> Result<()> {
        let _g = self.gate.write().await;
        self.ready()?;
        let src = mutable_node(old)?;
        let dst = mutable_node(new)?;
        let h = self.store.head(&src).await?;
        if src == dst {
            return Ok(());
        }
        if dst.starts_with(&format!("{src}/")) || src.starts_with(&format!("{dst}/")) {
            return Err(Error::invalid_path(new));
        }
        self.parent(&dst).await?;
        if replace {
            file(&h, old)?
        }
        let target = self.optional(&dst).await?;
        if let Some(t) = &target {
            if !replace {
                return Err(Error::AlreadyExists(new.into()));
            }
            file(t, new)?;
        }
        if let Some(dfs) = &self.store.dfs {
            return dfs
                .rename(&src, &dst, &h.tag, target.as_ref().map(|h| &h.tag))
                .await;
        }
        let m = Move {
            id: uuid::Uuid::new_v4().to_string(),
            src,
            dst,
            replace_tag: target.map(|x| x.tag.to_string()),
        };
        self.dirty.store(true, Ordering::Release);
        let tag = self
            .store
            .put(
                JOURNAL,
                serde_json::to_vec(&m)?.into(),
                Condition::New,
                HashMap::new(),
            )
            .await?;
        self.finish_move(&m).await?;
        self.store.delete(JOURNAL, &tag).await?;
        self.dirty.store(false, Ordering::Release);
        Ok(())
    }
    async fn compare(&self, path: &str, expected: &[u8], new: Option<&[u8]>) -> Result<bool> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = mutable_node(path)?;
        let result = async {
            let h = self.store.head(&k).await?;
            if info(&k, &h).is_dir
                || h.size != expected.len() as u64
                || self.store.get(&k, &h.tag, None).await?.as_ref() != expected
            {
                return Ok(false);
            }
            match new {
                Some(data) => {
                    self.store
                        .put(
                            &k,
                            Bytes::copy_from_slice(data),
                            Condition::Match(h.tag.clone()),
                            metadata(info(&k, &h).mode, "file"),
                        )
                        .await?;
                }
                None => self.store.delete(&k, &h.tag).await?,
            }
            Ok(true)
        }
        .await;
        match result {
            Err(Error::WouldBlock(_) | Error::NotFound(_)) => Ok(false),
            other => other,
        }
    }
}

#[async_trait]
impl FileSystem for AzureFS {
    async fn create(&self, p: &str) -> Result<()> {
        self.write(p, &[], 0, WriteFlag::CreateNew).await?;
        Ok(())
    }
    async fn mkdir(&self, p: &str, mode: u32) -> Result<()> {
        let _g = self.gate.write().await;
        self.ready()?;
        let k = mutable_node(p)?;
        self.parent(&k).await?;
        if let Some(dfs) = &self.store.dfs {
            return dfs.mkdir(&k, mode).await;
        }
        self.store
            .put(&k, Bytes::new(), Condition::New, metadata(mode, "dir"))
            .await
            .map(|_| ())
            .map_err(|e| match e {
                Error::WouldBlock(_) => Error::AlreadyExists(p.into()),
                e => e,
            })
    }
    async fn remove(&self, p: &str) -> Result<()> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = mutable_node(p)?;
        let h = self.store.head(&k).await?;
        file(&h, p)?;
        self.store.delete(&k, &h.tag).await
    }
    async fn remove_all(&self, p: &str) -> Result<()> {
        let _g = self.gate.write().await;
        self.ready()?;
        let k = mutable_node(p)?;
        let h = self.store.head(&k).await?;
        if info(&k, &h).is_dir {
            if let Some(dfs) = &self.store.dfs {
                return dfs.remove_tree(&k, &h.tag).await;
            }
            loop {
                let keys = self.store.key_batch(&format!("{k}/"), Some(128)).await?;
                if keys.is_empty() {
                    break;
                }
                for child in keys {
                    let t = self.store.head(&child).await?;
                    self.store.delete(&child, &t.tag).await?;
                }
            }
        }
        self.store.delete(&k, &h.tag).await
    }
    async fn stat(&self, p: &str) -> Result<FileInfo> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = node(p)?;
        if k == "nodes" {
            return Ok(FileInfo {
                name: "/".into(),
                size: 0,
                mode: 0o755,
                mod_time: SystemTime::UNIX_EPOCH,
                is_dir: true,
            });
        }
        Ok(info(&k, &self.store.head(&k).await?))
    }
    async fn read_dir(
        &self,
        p: &str,
        offset: Option<usize>,
        limit: Option<usize>,
        sort: Option<ListSortBy>,
        order: Option<SortOrder>,
    ) -> Result<Vec<FileInfo>> {
        self.read_dir_with_options(
            p,
            ReadDirOptions {
                offset,
                limit,
                sort_by: sort,
                sort_order: order,
                ..Default::default()
            },
        )
        .await
    }

    async fn read_dir_with_options(
        &self,
        p: &str,
        options: ReadDirOptions,
    ) -> Result<Vec<FileInfo>> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = node(p)?;
        if k != "nodes" && !info(&k, &self.store.head(&k).await?).is_dir {
            return Err(Error::NotADirectory(p.into()));
        }
        self.store.children(&format!("{k}/"), options).await
    }
    async fn read(&self, p: &str, offset: u64, size: u64) -> Result<Vec<u8>> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = mutable_node(p)?;
        let h = self.store.head(&k).await?;
        file(&h, p)?;
        let remaining = h.size.saturating_sub(offset);
        let len = if size == 0 {
            remaining
        } else {
            size.min(remaining)
        };
        if len == 0 {
            return Ok(vec![]);
        }
        Ok(self
            .store
            .get(&k, &h.tag, Some(HttpRange::new(offset, len)))
            .await?
            .to_vec())
    }
    async fn write(&self, p: &str, data: &[u8], offset: u64, flag: WriteFlag) -> Result<u64> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = mutable_node(p)?;
        self.parent(&k).await?;
        let h = self.optional(&k).await?;
        let (body, cond, mode) = match h {
            None if matches!(flag, WriteFlag::Create | WriteFlag::CreateNew) => {
                (Bytes::copy_from_slice(data), Condition::New, 0o644)
            }
            None => return Err(Error::not_found(p)),
            Some(h) => {
                if flag == WriteFlag::CreateNew {
                    return Err(Error::AlreadyExists(p.into()));
                }
                file(&h, p)?;
                let bytes = match flag {
                    WriteFlag::None | WriteFlag::Append => {
                        let mut body = self.store.get(&k, &h.tag, None).await?.to_vec();
                        let start = if flag == WriteFlag::Append {
                            body.len()
                        } else {
                            usize::try_from(offset)
                                .map_err(|_| Error::invalid_operation("offset too large"))?
                        };
                        let end = start
                            .checked_add(data.len())
                            .ok_or_else(|| Error::invalid_operation("offset overflow"))?;
                        if end > body.len() {
                            body.try_reserve(end - body.len()).map_err(|_| {
                                Error::invalid_operation("write exceeds memory limit")
                            })?;
                            body.resize(end, 0)
                        }
                        body[start..end].copy_from_slice(data);
                        Bytes::from(body)
                    }
                    _ => Bytes::copy_from_slice(data),
                };
                let mode = info(&k, &h).mode;
                (bytes, Condition::Match(h.tag), mode)
            }
        };
        match self.store.put(&k, body, cond, metadata(mode, "file")).await {
            Err(Error::WouldBlock(_)) if flag == WriteFlag::CreateNew => {
                Err(Error::AlreadyExists(p.into()))
            }
            res => res.map(|_| data.len() as u64),
        }
    }
    async fn compare_and_write(&self, p: &str, expected: &[u8], new: &[u8]) -> Result<bool> {
        self.compare(p, expected, Some(new)).await
    }
    async fn compare_and_remove(&self, p: &str, expected: &[u8]) -> Result<bool> {
        self.compare(p, expected, None).await
    }
    async fn rename(&self, a: &str, b: &str) -> Result<()> {
        self.move_node(a, b, false).await
    }
    async fn replace(&self, a: &str, b: &str) -> Result<()> {
        self.move_node(a, b, true).await
    }
    async fn truncate(&self, p: &str, size: u64) -> Result<()> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = mutable_node(p)?;
        let h = self.store.head(&k).await?;
        file(&h, p)?;
        let len = usize::try_from(size).map_err(|_| Error::invalid_operation("size too large"))?;
        let mut body = if len == 0 {
            Vec::new()
        } else {
            self.store.get(&k, &h.tag, None).await?.to_vec()
        };
        if len > body.len() {
            body.try_reserve(len - body.len())
                .map_err(|_| Error::invalid_operation("truncate exceeds memory limit"))?
        }
        body.resize(len, 0);
        let mode = info(&k, &h).mode;
        self.store
            .put(
                &k,
                body.into(),
                Condition::Match(h.tag),
                metadata(mode, "file"),
            )
            .await?;
        Ok(())
    }
    async fn chmod(&self, p: &str, mode: u32) -> Result<()> {
        let _g = self.gate.read().await;
        self.ready()?;
        let k = mutable_node(p)?;
        let h = self.store.head(&k).await?;
        self.store.set_mode(&k, h, mode).await
    }
}

#[cfg(test)]
mod tests {
    use super::store::Auth;
    use super::*;

    /// Seed the exact persisted states on either side of source deletion, then reopen.
    #[tokio::test]
    #[ignore = "requires disposable Blob container"]
    async fn azure_recover_interrupted_move() {
        for (deleted, changed) in [(false, false), (true, false), (false, true)] {
            let cfg = AzureConfig {
                namespace: Default::default(),
                account_url: std::env::var("OV_AZURE_TEST_URL").unwrap(),
                container: std::env::var("OV_AZURE_TEST_CONTAINER").unwrap(),
                prefix: format!("ov-tests/{}", uuid::Uuid::new_v4()),
                auth: if std::env::var_os("OV_AZURE_TEST_SAS").is_some() {
                    Auth::SasEnv {
                        env: "OV_AZURE_TEST_SAS".into(),
                    }
                } else {
                    Auth::ManagedIdentity {
                        client_id: std::env::var("AZURE_CLIENT_ID").ok(),
                    }
                },
            };
            let fs = AzureFS::open(&cfg).await.unwrap();
            fs.mkdir("/src", 0o755).await.unwrap();
            fs.write("/src/file", b"old", 0, WriteFlag::Create)
                .await
                .unwrap();
            let m = Move {
                id: "test-move".into(),
                src: "nodes/src".into(),
                dst: "nodes/dst".into(),
                replace_tag: None,
            };
            let h = fs.store.head("nodes/src/file").await.unwrap();
            let mut meta = h.meta.clone();
            meta.insert("moveid".into(), m.id.clone());
            meta.insert("sourcetag".into(), h.tag.to_string());
            fs.store
                .put(
                    JOURNAL,
                    serde_json::to_vec(&m).unwrap().into(),
                    Condition::New,
                    Default::default(),
                )
                .await
                .unwrap();
            let d = fs
                .store
                .copy("nodes/src/file", &h, "nodes/dst/file", Condition::New, meta)
                .await
                .unwrap();
            let page = MovePage {
                id: m.id.clone(),
                items: vec![MoveItem {
                    src: "nodes/src/file".into(),
                    tag: h.tag.to_string(),
                    published: if deleted || changed {
                        Some(d.to_string())
                    } else {
                        None
                    },
                }],
            };
            fs.store
                .put(
                    PAGE,
                    serde_json::to_vec(&page).unwrap().into(),
                    Condition::New,
                    Default::default(),
                )
                .await
                .unwrap();
            if deleted {
                fs.store.delete("nodes/src/file", &h.tag).await.unwrap();
            }
            if changed {
                fs.store
                    .put(
                        "nodes/dst/file",
                        Bytes::from_static(b"external"),
                        Condition::Match(d),
                        Default::default(),
                    )
                    .await
                    .unwrap();
                assert!(matches!(
                    AzureFS::open(&cfg).await,
                    Err(Error::WouldBlock(_))
                ));
                assert_eq!(fs.read("/src/file", 0, 0).await.unwrap(), b"old");
                continue;
            }
            drop(fs);
            let fresh = AzureFS::open(&cfg).await.unwrap();
            assert!(fresh.stat("/dst").await.unwrap().is_dir);
            assert_eq!(fresh.read("/dst/file", 0, 0).await.unwrap(), b"old");
            assert!(matches!(fresh.stat("/src").await, Err(Error::NotFound(_))));
            assert!(fresh.optional(JOURNAL).await.unwrap().is_none());
        }
    }
}
