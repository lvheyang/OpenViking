//! Common Azure SDK operations. Conditional failures stay distinct from transport errors.
use super::hns::DataLake;
use crate::core::{Error, FileInfo, ReadDirOptions, Result};
use azure_core::{
    credentials::TokenCredential,
    error::ErrorKind,
    http::{Etag, Url},
};
use azure_identity::{
    AzureCliCredential, AzureCliCredentialOptions, ManagedIdentityCredential,
    ManagedIdentityCredentialOptions, UserAssignedId,
};
use azure_storage_blob::{models::*, BlobContainerClient};
use bytes::Bytes;
use futures::{stream, StreamExt, TryStreamExt};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc, time::SystemTime};
use tokio::sync::OnceCell;

/// Namespace semantics required of the storage account.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Namespace {
    /// Prefixes and explicit directory markers.
    #[default]
    Flat,
    /// Service-managed Data Lake directories.
    Hns,
}

/// Location and identity for a pre-created Blob container.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AzureConfig {
    /// Defaults to flat for existing configuration files.
    #[serde(default)]
    pub namespace: Namespace,
    /// Unsigned account endpoint.
    pub account_url: String,
    /// Pre-created Blob container.
    pub container: String,
    /// Exclusive namespace inside the container.
    pub prefix: String,
    /// Runtime authentication source.
    pub auth: Auth,
}

/// Credentials are resolved at runtime; configuration never carries a token.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Auth {
    /// Explicit local development login from Azure CLI.
    AzureCli {
        /// Select the CLI account without changing its global default subscription.
        subscription: Option<String>,
    },
    /// System-assigned or user-assigned Azure managed identity.
    ManagedIdentity {
        /// Client ID of the user-assigned identity, when selected.
        client_id: Option<String>,
    },
    /// SAS for a loopback emulator.
    SasEnv {
        /// Name of the environment variable holding the SAS.
        env: String,
    },
}

pub(super) fn service_error(err: azure_core::Error) -> Error {
    // SDK error chains can contain the signed request URL.
    if let ErrorKind::HttpResponse {
        status, error_code, ..
    } = err.kind()
    {
        let code = error_code.as_deref().unwrap_or("unspecified");
        let msg = format!("Blob HTTP {status} ({code})");
        return match (u16::from(*status), code) {
            (
                404,
                "BlobNotFound"
                | "PathNotFound"
                | "SourcePathNotFound"
                | "RenameDestinationParentPathNotFound"
                | "unspecified",
            ) => Error::NotFound(msg),
            (
                412,
                "ConditionNotMet"
                | "TargetConditionNotMet"
                | "SourceConditionNotMet"
                | "unspecified",
            )
            | (409, "BlobAlreadyExists" | "PathAlreadyExists") => Error::WouldBlock(msg),
            (401 | 403, _) => Error::PermissionDenied(msg),
            (429, _) => Error::RateLimited(msg),
            _ => Error::Network(msg),
        };
    }
    Error::Network("Azure SDK request failed before a service response".into())
}

pub(super) fn valid_path(s: &str) -> bool {
    !s.contains('\\')
        && !s.chars().any(char::is_control)
        && s.split('/').all(|p| !matches!(p, "" | "." | ".."))
}

// SDK 1.1 exposes flat List Blobs. Set the standard delimiter on directory-list
// requests only, preserving the SDK pager, authorization, retries and error handling.
#[derive(Debug, Clone)]
struct DirectoryList;
#[async_trait::async_trait]
impl azure_core::http::policies::Policy for DirectoryList {
    async fn send(
        &self,
        ctx: &azure_core::http::Context,
        req: &mut azure_core::http::Request,
        next: &[Arc<dyn azure_core::http::policies::Policy>],
    ) -> azure_core::http::policies::PolicyResult {
        if ctx.value::<DirectoryList>().is_some() {
            req.url_mut()
                .query_pairs_mut()
                .append_pair("delimiter", "/");
        }
        next[0].send(ctx, req, &next[1..]).await
    }
}

/// Shared SDK client scoped to one application prefix.
pub struct BlobStore {
    pub(super) dfs: Option<DataLake>,
    checked: OnceCell<()>,
    flat_emulator: bool,
    client: BlobContainerClient,
    prefix: String,
}

pub(crate) struct Head {
    pub tag: Etag,
    pub size: u64,
    pub time: SystemTime,
    pub meta: HashMap<String, String>,
}

pub(crate) enum Condition {
    New,
    Match(Etag),
    Any,
}

impl BlobStore {
    /// Construct a client scoped to the configured container and prefix.
    pub fn new(config: &AzureConfig) -> Result<Self> {
        let invalid = || Error::Config("invalid Azure account URL, container or prefix".into());
        let mut url = Url::parse(&config.account_url).map_err(|_| invalid())?;
        let c = &config.container;
        if !valid_path(&config.prefix)
            || !(3..=63).contains(&c.len())
            || c.starts_with('-')
            || c.ends_with('-')
            || c.contains("--")
            || !c
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || url.host_str().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
        let credential: Option<Arc<dyn TokenCredential>> = match &config.auth {
            Auth::AzureCli { subscription } => {
                if url.scheme() != "https" || !url.path().trim_matches('/').is_empty() {
                    return Err(invalid());
                }
                Some(
                    AzureCliCredential::new(Some(AzureCliCredentialOptions {
                        subscription: subscription.clone(),
                        ..Default::default()
                    }))
                    .map_err(service_error)?,
                )
            }
            Auth::ManagedIdentity { client_id } => {
                if url.scheme() != "https" || !url.path().trim_matches('/').is_empty() {
                    return Err(invalid());
                }
                Some(
                    ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
                        user_assigned_id: client_id.clone().map(UserAssignedId::ClientId),
                        ..Default::default()
                    }))
                    .map_err(service_error)?,
                )
            }
            Auth::SasEnv { env } => {
                if !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
                    || !matches!(url.scheme(), "http" | "https")
                    || env.is_empty()
                    || !env.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err(invalid());
                }
                let token = std::env::var(env).map_err(|_| {
                    Error::Config("missing emulator SAS environment variable".into())
                })?;
                url.set_query(Some(token.trim().trim_start_matches('?')));
                None
            }
        };
        url.path_segments_mut()
            .map_err(|_| invalid())?
            .pop_if_empty()
            .push(c);
        let mut options = azure_storage_blob::BlobContainerClientOptions::default();
        options
            .client_options
            .per_call_policies
            .push(Arc::new(DirectoryList));
        let dfs = match config.namespace {
            Namespace::Flat => None,
            Namespace::Hns => Some(DataLake::new(
                url.clone(),
                config.prefix.clone(),
                credential.clone(),
            )?),
        };
        Ok(Self {
            dfs,
            checked: OnceCell::new(),
            flat_emulator: matches!(config.auth, Auth::SasEnv { .. })
                && config.namespace == Namespace::Flat,
            client: BlobContainerClient::new(url, credential, Some(options))
                .map_err(service_error)?,
            prefix: format!("{}/", config.prefix),
        })
    }

    pub(crate) async fn verify_namespace(&self) -> Result<()> {
        self.checked
            .get_or_try_init(|| async {
                if !self.flat_emulator {
                    let enabled = match self.client.get_account_info(None).await {
                        Ok(response) => response
                            .is_hierarchical_namespace_enabled()
                            .map_err(service_error)?,
                        // getInfo/action is independent from the data-plane contributor role.
                        // Preserve existing flat accounts that only grant container data access.
                        Err(e)
                            if e.http_status() == Some(azure_core::http::StatusCode::Forbidden) =>
                        {
                            if let Some(dfs) = &self.dfs {
                                dfs.verify_acl_support().await?;
                            }
                            return Ok(());
                        }
                        Err(e) => return Err(service_error(e)),
                    };
                    if enabled != Some(self.dfs.is_some()) {
                        return Err(Error::Config(
                            "configured Azure namespace does not match account HNS capability"
                                .into(),
                        ));
                    }
                }
                Ok(())
            })
            .await
            .copied()
    }

    pub(crate) async fn put_git(
        &self,
        key: &str,
        body: Bytes,
        condition: Condition,
    ) -> Result<Etag> {
        self.verify_namespace().await?;
        if let Some(dfs) = &self.dfs {
            let (parent, _) = key
                .rsplit_once('/')
                .ok_or_else(|| Error::invalid_path(key))?;
            dfs.ensure_directories(parent).await?;
        }
        self.put(key, body, condition, Default::default()).await
    }

    fn blob(&self, key: &str) -> azure_storage_blob::BlobClient {
        self.client.blob_client(&format!("{}{key}", self.prefix))
    }

    pub(crate) async fn head(&self, key: &str) -> Result<Head> {
        self.verify_namespace().await?;
        if let Some(dfs) = &self.dfs {
            return dfs.head(key).await;
        }
        let r = self
            .blob(key)
            .get_properties(None)
            .await
            .map_err(service_error)?;
        let missing = || Error::Network("incomplete Blob properties".into());
        Ok(Head {
            tag: r.etag().map_err(service_error)?.ok_or_else(missing)?,
            size: r
                .content_length()
                .map_err(service_error)?
                .ok_or_else(missing)?,
            time: r
                .last_modified()
                .map_err(service_error)?
                .ok_or_else(missing)?
                .into(),
            meta: r.metadata().map_err(service_error)?,
        })
    }

    pub(crate) async fn get(
        &self,
        key: &str,
        tag: &Etag,
        range: Option<HttpRange>,
    ) -> Result<Bytes> {
        let r = self
            .blob(key)
            .download(Some(BlobClientDownloadOptions {
                if_match: Some(tag.clone()),
                range,
                ..Default::default()
            }))
            .await
            .map_err(service_error)?;
        r.body.collect().await.map_err(service_error)
    }

    pub(crate) async fn put(
        &self,
        key: &str,
        body: Bytes,
        cond: Condition,
        mut meta: HashMap<String, String>,
    ) -> Result<Etag> {
        let operation = uuid::Uuid::new_v4().to_string();
        meta.insert("writeid".into(), operation.clone());
        let mut opts = BlobClientUploadOptions {
            metadata: Some(meta),
            ..Default::default()
        };
        match cond {
            Condition::New => opts.if_none_match = Some("*".into()),
            Condition::Match(tag) => opts.if_match = Some(tag),
            Condition::Any => {}
        }
        match self
            .blob(key)
            .upload(body.into(), Some(opts))
            .await
            .map_err(service_error)
        {
            Ok(r) => r
                .etag
                .ok_or_else(|| Error::Network("missing upload ETag".into())),
            Err(e @ (Error::WouldBlock(_) | Error::Network(_))) => {
                // A retry after a lost response can fail its original precondition.
                // Only acknowledge our own write; never replay a read-modify-write.
                if let Ok(h) = self.head(key).await {
                    if h.meta.get("writeid") == Some(&operation) {
                        return Ok(h.tag);
                    }
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn set_mode(&self, key: &str, head: Head, mode: u32) -> Result<()> {
        let mut metadata = head.meta;
        metadata.insert("mode".into(), mode.to_string());
        self.blob(key)
            .set_metadata(
                &metadata,
                Some(BlobClientSetMetadataOptions {
                    if_match: Some(head.tag),
                    ..Default::default()
                }),
            )
            .await
            .map_err(service_error)?;
        Ok(())
    }

    pub(crate) async fn delete(&self, key: &str, tag: &Etag) -> Result<()> {
        self.blob(key)
            .delete(Some(BlobClientDeleteOptions {
                if_match: Some(tag.clone()),
                ..Default::default()
            }))
            .await
            .map_err(service_error)?;
        Ok(())
    }

    /// Copy from a pinned generation. Publish the staged blocks in one conditional commit.
    pub(crate) async fn copy(
        &self,
        src: &str,
        h: &Head,
        dst: &str,
        cond: Condition,
        meta: HashMap<String, String>,
    ) -> Result<Etag> {
        const CHUNK: u64 = 8 * 1024 * 1024;
        if h.size <= CHUNK {
            let bytes = self.get(src, &h.tag, None).await?;
            return self.put(dst, bytes, cond, meta).await;
        }
        let chunk = CHUNK.max(h.size.div_ceil(50_000));
        let client = self.blob(dst).block_blob_client();
        let mut blocks = Vec::new();
        let mut offset = 0;
        while offset < h.size {
            let size = chunk.min(h.size - offset);
            let body = self
                .get(src, &h.tag, Some(HttpRange::new(offset, size)))
                .await?;
            let id = uuid::Uuid::new_v4();
            client
                .stage_block(id.as_bytes(), size, body.into(), None)
                .await
                .map_err(service_error)?;
            blocks.push(id.as_bytes().to_vec());
            offset += size;
        }
        let mut opts = BlockBlobClientCommitBlockListOptions {
            metadata: Some(meta),
            ..Default::default()
        };
        match cond {
            Condition::New => opts.if_none_match = Some("*".into()),
            Condition::Match(tag) => opts.if_match = Some(tag),
            Condition::Any => {}
        }
        let list = BlockLookupList {
            latest: Some(blocks),
            ..Default::default()
        };
        client
            .commit_block_list(list.try_into().map_err(service_error)?, Some(opts))
            .await
            .map_err(service_error)?
            .etag()
            .map_err(service_error)?
            .ok_or_else(|| Error::Network("missing copy ETag".into()))
    }

    pub(crate) async fn children(
        &self,
        prefix: &str,
        options: ReadDirOptions,
    ) -> Result<Vec<FileInfo>> {
        if let Some(dfs) = &self.dfs {
            if options.limit == Some(0) {
                return Ok(Vec::new());
            }
            let listed = dfs.list(prefix.trim_end_matches('/'), false).await?;
            let mut keys = HashMap::with_capacity(listed.len());
            let entries = listed
                .into_iter()
                .map(|(key, info)| {
                    keys.insert(info.name.clone(), key);
                    info
                })
                .collect();
            let page = options.apply(entries);
            // Preserve the selected order, with at most eight metadata requests in flight.
            // A disappearing entry is omitted, as with other non-snapshot directory reads.
            return stream::iter(page)
                .map(|entry| {
                    let key = &keys[&entry.name];
                    async move {
                        match dfs.head(key).await {
                            Ok(h) => Ok(Some(super::info(key, &h))),
                            Err(Error::NotFound(_)) => Ok(None),
                            Err(e) => Err(e),
                        }
                    }
                })
                .buffered(8)
                .try_filter_map(|entry| async { Ok(entry) })
                .try_collect()
                .await;
        }
        let mut opts = BlobContainerClientListBlobsOptions {
            prefix: Some(format!("{}{prefix}", self.prefix)),
            include: Some(vec![ListBlobsIncludeItem::Metadata]),
            maxresults: Some(128),
            ..Default::default()
        };
        opts.method_options.context.insert(DirectoryList);
        let mut pager = self.client.list_blobs(Some(opts)).map_err(service_error)?;
        let mut entries = Vec::new();
        let full_prefix = format!("{}{prefix}", self.prefix);
        while let Some(item) = pager.try_next().await.map_err(service_error)? {
            let invalid = || Error::Network("incomplete directory list response".into());
            let name = item
                .name
                .as_deref()
                .and_then(|n| n.strip_prefix(&full_prefix))
                .filter(|n| !n.contains('/'))
                .ok_or_else(invalid)?;
            let props = item.properties.ok_or_else(invalid)?;
            let meta = item.metadata.and_then(|m| m.values).unwrap_or_default();
            entries.push(super::file_info(
                name,
                props.content_length.ok_or_else(invalid)?,
                props.last_modified.ok_or_else(invalid)?.into(),
                &meta,
            ));
        }
        Ok(options.apply(entries))
    }

    pub(crate) async fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        self.verify_namespace().await?;
        if let Some(dfs) = &self.dfs {
            return match dfs.list(prefix.trim_end_matches('/'), true).await {
                Ok(entries) => Ok(entries
                    .into_iter()
                    .filter_map(|(k, info)| (!info.is_dir).then_some(k))
                    .collect()),
                Err(Error::NotFound(_)) => Ok(vec![]),
                Err(e) => Err(e),
            };
        }
        self.key_batch(prefix, None).await
    }

    pub(crate) async fn key_batch(&self, prefix: &str, limit: Option<i32>) -> Result<Vec<String>> {
        let mut pager = self
            .client
            .list_blobs(Some(BlobContainerClientListBlobsOptions {
                maxresults: limit,
                prefix: Some(format!("{}{prefix}", self.prefix)),
                ..Default::default()
            }))
            .map_err(service_error)?;
        let mut keys = Vec::new();
        while let Some(item) = pager.try_next().await.map_err(service_error)? {
            let name = item
                .name
                .as_deref()
                .and_then(|n| n.strip_prefix(&self.prefix))
                .filter(|n| n.starts_with(prefix))
                .ok_or_else(|| Error::Network("unexpected blob name in listing".into()))?;
            keys.push(name.to_owned());
            if limit.is_some_and(|n| keys.len() == n as usize) {
                break;
            }
        }
        Ok(keys)
    }
}
