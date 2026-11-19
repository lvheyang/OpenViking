//! Data Lake Gen2 namespace operations. File bytes stay on the Block Blob data path.
use super::store::{service_error, Head};
use crate::core::{Error, FileInfo, Result};
use azure_core::{
    base64,
    credentials::TokenCredential,
    http::{
        policies::{auth::BearerTokenAuthorizationPolicy, Policy},
        Context, Etag, Method, Pipeline, RawResponse, Request, Url,
    },
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};

pub(super) struct DataLake {
    container: Url,
    prefix: String,
    pipeline: Pipeline,
}

fn header(response: &RawResponse, name: &'static str) -> Option<String> {
    response.headers().get_optional_string(&name.into())
}
fn required(response: &RawResponse, name: &'static str) -> Result<String> {
    header(response, name).ok_or_else(|| Error::Network(format!("DFS response missing {name}")))
}
fn is_dir(response: &RawResponse) -> Result<bool> {
    match required(response, "x-ms-resource-type")?.as_str() {
        "directory" => Ok(true),
        "file" => Ok(false),
        _ => Err(Error::Network("invalid DFS resource type".into())),
    }
}

impl DataLake {
    pub(super) fn new(
        mut container: Url,
        prefix: String,
        credential: Option<Arc<dyn TokenCredential>>,
    ) -> Result<Self> {
        if credential.is_some() {
            // Standard Azure, sovereign clouds, and the standard hostname backed by Private DNS.
            let (account, suffix) = container
                .host_str()
                .and_then(|host| host.split_once(".blob."))
                .filter(|(account, _)| !account.contains('.'))
                .ok_or_else(|| {
                    Error::Config("HNS requires an Azure account Blob endpoint".into())
                })?;
            container
                .set_host(Some(&format!("{account}.dfs.{suffix}")))
                .map_err(|_| Error::Config("invalid DFS endpoint".into()))?;
        }
        let mut auth: Vec<Arc<dyn Policy>> = Vec::new();
        if let Some(c) = credential {
            auth.push(Arc::new(BearerTokenAuthorizationPolicy::new(
                c,
                vec!["https://storage.azure.com/.default"],
            )));
        }
        let pipeline = Pipeline::new(
            Some("ragfs"),
            Some(env!("CARGO_PKG_VERSION")),
            Default::default(),
            vec![],
            auth,
            None,
        );
        Ok(Self {
            container,
            prefix,
            pipeline,
        })
    }

    fn url(&self, path: &str) -> Url {
        let mut url = self.container.clone();
        if !path.is_empty() {
            url.path_segments_mut()
                .expect("validated container")
                .extend(path.split('/'));
        }
        url
    }

    fn full(&self, key: &str) -> String {
        format!("{}/{key}", self.prefix)
    }

    fn request(&self, path: &str, method: Method) -> Request {
        let mut req = Request::new(self.url(path), method);
        req.insert_header("x-ms-version", "2023-11-03");
        req
    }
    async fn send(&self, mut req: Request) -> Result<RawResponse> {
        self.pipeline
            .send(&Context::default(), &mut req, None)
            .await
            .map_err(service_error)
    }

    pub(super) async fn verify_acl_support(&self) -> Result<()> {
        let mut request = self.request("", Method::Head);
        request
            .url_mut()
            .path_segments_mut()
            .expect("validated container")
            .push("");
        request
            .url_mut()
            .query_pairs_mut()
            .append_pair("action", "getAccessControl");
        let response = self.send(request).await?;
        required(&response, "x-ms-permissions")?;
        Ok(())
    }

    async fn properties(&self, path: &str) -> Result<RawResponse> {
        self.send(self.request(path, Method::Head)).await
    }
    pub(super) async fn head(&self, key: &str) -> Result<Head> {
        let response = self.properties(&self.full(key)).await?;
        let directory = is_dir(&response)?;
        let mut meta = HashMap::new();
        for field in header(&response, "x-ms-properties")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
        {
            let (name, encoded) = field
                .trim()
                .split_once('=')
                .ok_or_else(|| Error::Network("malformed DFS properties".into()))?;
            if matches!(name, "mode" | "kind" | "writeid") {
                let value = String::from_utf8(base64::decode(encoded).map_err(service_error)?)
                    .map_err(|_| Error::Network("invalid OV property".into()))?;
                meta.insert(name.to_owned(), value);
            }
        }
        if !directory && meta.get("kind").is_some_and(|kind| kind == "dir") {
            return Err(Error::Config(
                "flat directory marker in HNS namespace; migrate to a fresh prefix".into(),
            ));
        }
        meta.insert("kind".into(), if directory { "dir" } else { "file" }.into());
        let time = chrono::DateTime::parse_from_rfc2822(&required(&response, "last-modified")?)
            .map_err(|_| Error::Network("invalid DFS last-modified".into()))?;
        Ok(Head {
            tag: required(&response, "etag")?.into(),
            size: if directory {
                0
            } else {
                required(&response, "content-length")?
                    .parse()
                    .map_err(|_| Error::Network("invalid DFS size".into()))?
            },
            time: time.with_timezone(&chrono::Utc).into(),
            meta,
        })
    }

    async fn create_directory(&self, path: &str, mode: u32) -> Result<()> {
        let mut req = self.request(path, Method::Put);
        req.url_mut()
            .query_pairs_mut()
            .append_pair("resource", "directory");
        req.insert_header("if-none-match", "*");
        req.insert_header(
            "x-ms-properties",
            format!("mode={}", base64::encode(mode.to_string())),
        );
        self.send(req).await.map(|_| ()).map_err(|e| match e {
            Error::WouldBlock(_) => Error::AlreadyExists(path.into()),
            other => other,
        })
    }

    pub(super) async fn mkdir(&self, key: &str, mode: u32) -> Result<()> {
        self.create_directory(&self.full(key), mode).await
    }

    pub(super) async fn ensure_directories(&self, key: &str) -> Result<()> {
        let full = self.full(key);
        let mut path = String::new();
        for part in full.split('/') {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            match self.properties(&path).await {
                Ok(r) if is_dir(&r)? => continue,
                Ok(_) => return Err(Error::NotADirectory(path)),
                Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
            match self.create_directory(&path, 0o755).await {
                Ok(()) => {}
                Err(Error::AlreadyExists(_)) if is_dir(&self.properties(&path).await?)? => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub(super) async fn list(&self, key: &str, recursive: bool) -> Result<Vec<(String, FileInfo)>> {
        #[derive(Deserialize)]
        struct Page {
            paths: Vec<Entry>,
        }
        // The REST service represents isDirectory as a string, and omits it for files.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Flag {
            Bool(bool),
            Text(String),
        }
        #[derive(Deserialize)]
        struct Entry {
            name: String,
            #[serde(rename = "isDirectory")]
            directory: Option<Flag>,
            #[serde(rename = "lastModified")]
            modified: String,
        }
        let path = self.full(key);
        let parent = format!("{path}/");
        let mut entries = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut req = self.request("", Method::Get);
            req.url_mut()
                .query_pairs_mut()
                .append_pair("resource", "filesystem")
                .append_pair("directory", &path)
                .append_pair("recursive", if recursive { "true" } else { "false" })
                .append_pair("maxResults", "128");
            if let Some(t) = &token {
                req.url_mut()
                    .query_pairs_mut()
                    .append_pair("continuation", t);
            }
            let r = self.send(req).await?;
            for item in serde_json::from_slice::<Page>(r.body().as_ref())
                .map_err(|_| Error::Network("invalid DFS listing".into()))?
                .paths
            {
                let relative = item
                    .name
                    .strip_prefix(&parent)
                    .filter(|s| super::store::valid_path(s) && (recursive || !s.contains('/')))
                    .ok_or_else(|| Error::Network("invalid DFS child path".into()))?;
                let dir = match item.directory {
                    Some(Flag::Bool(v)) => v,
                    Some(Flag::Text(v)) => v
                        .parse()
                        .map_err(|_| Error::Network("invalid DFS directory flag".into()))?,
                    None => false,
                };
                let time = chrono::DateTime::parse_from_rfc2822(&item.modified)
                    .map_err(|_| Error::Network("invalid DFS lastModified".into()))?
                    .into();
                let child_key = format!("{key}/{relative}");
                let meta =
                    HashMap::from([("kind".into(), if dir { "dir" } else { "file" }.into())]);
                // Listing properties suffice for global sorting. Virtual mode and current
                // size are fetched only for the page returned to the caller.
                let info = super::file_info(&child_key, 0, time, &meta);
                entries.push((child_key, info));
            }
            token = next_page(&r, token.as_deref())?;
            if token.is_none() {
                return Ok(entries);
            }
        }
    }

    pub(super) async fn rename(
        &self,
        src: &str,
        dst: &str,
        source: &Etag,
        target: Option<&Etag>,
    ) -> Result<()> {
        let src = self.full(src);
        let dst = self.full(dst);
        let source_url = self.url(&src);
        // x-ms-rename-source always starts with the filesystem, including on a path-style emulator.
        let container = self
            .container
            .path_segments()
            .and_then(|mut s| s.next_back())
            .expect("validated container");
        let encoded = source_url
            .path()
            .strip_prefix(self.container.path())
            .expect("same container");
        let rename_source = format!("/{container}{encoded}");
        let transaction = uuid::Uuid::new_v4().to_string();
        let mut token: Option<String> = None;
        loop {
            let mut req = self.request(&dst, Method::Put);
            req.url_mut().query_pairs_mut().append_pair("mode", "posix");
            req.insert_header("x-ms-rename-source", &rename_source);
            req.insert_header("x-ms-client-transaction-id", &transaction);
            if let Some(t) = &token {
                req.url_mut()
                    .query_pairs_mut()
                    .append_pair("continuation", t);
            } else {
                req.insert_header("x-ms-source-if-match", source.to_string());
                match target {
                    Some(tag) => req.insert_header("if-match", tag.to_string()),
                    None => req.insert_header("if-none-match", "*"),
                }
            }
            let r = match self.send(req).await {
                Ok(r) => r,
                Err(error) => {
                    // Check precisely which rename was published after a lost acknowledgement.
                    if token.is_none()
                        && matches!(
                            error,
                            Error::NotFound(_) | Error::WouldBlock(_) | Error::Network(_)
                        )
                        && matches!(self.properties(&src).await, Err(Error::NotFound(_)))
                        && self
                            .properties(&dst)
                            .await
                            .ok()
                            .and_then(|r| header(&r, "x-ms-client-transaction-id"))
                            .as_ref()
                            == Some(&transaction)
                    {
                        return Ok(());
                    }
                    return Err(error);
                }
            };
            token = next_page(&r, token.as_deref())?;
            if token.is_none() {
                return Ok(());
            }
        }
    }

    pub(super) async fn remove_tree(&self, key: &str, tag: &Etag) -> Result<()> {
        let path = self.full(key);
        let mut token: Option<String> = None;
        loop {
            let mut req = self.request(&path, Method::Delete);
            req.url_mut()
                .query_pairs_mut()
                .append_pair("recursive", "true")
                .append_pair("paginated", "true");
            if let Some(t) = &token {
                req.url_mut()
                    .query_pairs_mut()
                    .append_pair("continuation", t);
            } else {
                req.insert_header("if-match", tag.to_string());
            }
            let r = self.send(req).await?;
            token = next_page(&r, token.as_deref())?;
            if token.is_none() {
                return Ok(());
            }
        }
    }
}

fn next_page(r: &RawResponse, last: Option<&str>) -> Result<Option<String>> {
    let token = header(r, "x-ms-continuation").filter(|s| !s.is_empty());
    if token.is_some() && token.as_deref() == last {
        return Err(Error::Network(
            "DFS returned a repeated continuation".into(),
        ));
    }
    Ok(token)
}
