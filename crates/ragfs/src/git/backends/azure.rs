//! Git loose objects, compare-and-swap references and rebuildable commit indexes on Azure.
use crate::core::Error;
use crate::git::{
    error::{ObjectStoreError, RefStoreError},
    index_store::{
        decode_index, encode_index, systemtime_to_ns, CommitIndex, IndexStore, IndexStoreError,
    },
    object_store::ObjectStore,
    ref_store::RefStore,
    util::validate_ref_name,
};
use crate::plugins::azureblobfs::store::{BlobStore, Condition};
use async_trait::async_trait;
use azure_core::http::Etag;
use bytes::Bytes;
use gix_hash::ObjectId;

fn account(a: &str) -> bool {
    !a.is_empty()
        && !matches!(a, "." | "..")
        && !a.contains(['/', '\\'])
        && !a.chars().any(char::is_control)
}
fn object_key(a: &str, oid: &ObjectId) -> Result<String, ObjectStoreError> {
    if !account(a) {
        return Err(ObjectStoreError::Backend("invalid account".into()));
    }
    let hex = oid.to_string();
    Ok(format!("{a}/objects/{}/{}", &hex[..2], &hex[2..]))
}
fn object_err(e: Error, oid: &ObjectId) -> ObjectStoreError {
    match e {
        Error::NotFound(_) => ObjectStoreError::NotFound(*oid),
        e => ObjectStoreError::Backend(e.to_string()),
    }
}
#[async_trait]
impl ObjectStore for BlobStore {
    async fn put(&self, a: &str, oid: &ObjectId, body: Bytes) -> Result<(), ObjectStoreError> {
        let key = object_key(a, oid)?;
        match self.put_git(&key, body, Condition::New).await {
            Ok(_) | Err(Error::WouldBlock(_)) => Ok(()),
            Err(e) => Err(object_err(e, oid)),
        }
    }
    async fn get(&self, a: &str, oid: &ObjectId) -> Result<Bytes, ObjectStoreError> {
        self.get_limited(a, oid, u64::MAX).await
    }
    async fn get_limited(
        &self,
        a: &str,
        oid: &ObjectId,
        limit: u64,
    ) -> Result<Bytes, ObjectStoreError> {
        let key = object_key(a, oid)?;
        let h = self.head(&key).await.map_err(|e| object_err(e, oid))?;
        if h.size > limit {
            return Err(ObjectStoreError::ReadLimitExceeded {
                size: h.size,
                limit,
            });
        }
        BlobStore::get(self, &key, &h.tag, None)
            .await
            .map_err(|e| object_err(e, oid))
    }
    async fn exists(&self, a: &str, oid: &ObjectId) -> Result<bool, ObjectStoreError> {
        match self.head(&object_key(a, oid)?).await {
            Ok(_) => Ok(true),
            Err(Error::NotFound(_)) => Ok(false),
            Err(e) => Err(object_err(e, oid)),
        }
    }
}

fn ref_key(a: &str, name: &str) -> Result<String, RefStoreError> {
    validate_ref_name(name)?;
    if !account(a)
        || !name.starts_with("refs/")
        || name.contains('\\')
        || name.split('/').any(|p| p.is_empty() || p == ".")
    {
        return Err(RefStoreError::InvalidName(name.into()));
    }
    Ok(format!("{a}/{name}"))
}
fn ref_err(e: Error) -> RefStoreError {
    RefStoreError::Backend(e.to_string())
}
impl BlobStore {
    async fn read_ref(&self, key: &str) -> Result<Option<(ObjectId, Etag)>, RefStoreError> {
        // A writer can win between HEAD and conditional GET. Read one newer generation.
        for _ in 0..4 {
            let h = match self.head(key).await {
                Ok(h) => h,
                Err(Error::NotFound(_)) => return Ok(None),
                Err(e) => return Err(ref_err(e)),
            };
            if h.size > 41 {
                return Err(RefStoreError::Backend(
                    "invalid Git reference length".into(),
                ));
            }
            match BlobStore::get(self, key, &h.tag, None).await {
                Ok(b) => {
                    return Ok(Some((
                        ObjectId::from_hex(b.trim_ascii_end())
                            .map_err(|_| RefStoreError::Backend("invalid Git reference".into()))?,
                        h.tag,
                    )))
                }
                Err(Error::WouldBlock(_) | Error::NotFound(_)) => continue,
                Err(e) => return Err(ref_err(e)),
            }
        }
        Err(RefStoreError::Backend(
            "reference changed during repeated reads".into(),
        ))
    }
}
#[async_trait]
impl RefStore for BlobStore {
    async fn read(&self, a: &str, n: &str) -> Result<ObjectId, RefStoreError> {
        self.read_ref(&ref_key(a, n)?)
            .await?
            .map(|(oid, _)| oid)
            .ok_or_else(|| RefStoreError::NotFound(n.into()))
    }
    async fn cas_update(
        &self,
        a: &str,
        n: &str,
        expected: Option<ObjectId>,
        new: ObjectId,
    ) -> Result<(), RefStoreError> {
        let k = ref_key(a, n)?;
        let cond = if expected.is_some() {
            let current = self.read_ref(&k).await?;
            match current {
                Some((oid, tag)) if Some(oid) == expected => Condition::Match(tag),
                other => {
                    return Err(RefStoreError::Conflict {
                        expected,
                        actual: other.map(|(oid, _)| oid),
                    })
                }
            }
        } else {
            Condition::New
        };
        match self.put_git(&k, format!("{new}\n").into(), cond).await {
            Ok(_) => Ok(()),
            Err(Error::WouldBlock(_) | Error::NotFound(_)) => Err(RefStoreError::Conflict {
                expected,
                actual: self.read_ref(&k).await?.map(|(oid, _)| oid),
            }),
            Err(e) => Err(ref_err(e)),
        }
    }
    async fn list(&self, a: &str, prefix: &str) -> Result<Vec<(String, ObjectId)>, RefStoreError> {
        let base = if matches!(prefix, "" | "refs" | "refs/") && account(a) {
            format!("{a}/refs/")
        } else {
            format!("{}/", ref_key(a, prefix.trim_end_matches('/'))?)
        };
        let mut out = Vec::new();
        for key in self.keys(&base).await.map_err(ref_err)? {
            if let Some((oid, _)) = self.read_ref(&key).await? {
                out.push((key[a.len() + 1..].into(), oid))
            }
        }
        Ok(out)
    }
}
fn index_key(a: &str, branch: &str) -> Result<String, IndexStoreError> {
    if !account(a)
        || branch.contains('\\')
        || branch.split('/').any(|p| p.is_empty() || p == ".")
        || validate_ref_name(branch).is_err()
    {
        return Err(IndexStoreError::InvalidBranch(branch.into()));
    }
    Ok(format!("{a}/index/{branch}.json"))
}
#[async_trait]
impl IndexStore for BlobStore {
    async fn load(&self, a: &str, branch: &str) -> Result<Option<CommitIndex>, IndexStoreError> {
        let k = index_key(a, branch)?;
        let h = match self.head(&k).await {
            Ok(h) => h,
            Err(Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(IndexStoreError::Backend(e.to_string())),
        };
        let b = BlobStore::get(self, &k, &h.tag, None)
            .await
            .map_err(|e| IndexStoreError::Backend(e.to_string()))?;
        let mut idx = decode_index(&b).unwrap_or(None);
        if let Some(i) = &mut idx {
            i.saved_at_ns = systemtime_to_ns(h.time)
        }
        Ok(idx)
    }
    async fn save(
        &self,
        a: &str,
        branch: &str,
        index: &CommitIndex,
    ) -> Result<(), IndexStoreError> {
        self.put_git(
            &index_key(a, branch)?,
            encode_index(index)?.into(),
            Condition::Any,
        )
        .await
        .map(|_| ())
        .map_err(|e| IndexStoreError::Backend(e.to_string()))
    }
}
