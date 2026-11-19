//! Run explicitly against a disposable pre-created flat or HNS Blob container.
#![cfg(feature = "azure")]
use ragfs::core::{Error, FileSystem, ListSortBy, SortOrder, WriteFlag};
use ragfs::git::{
    error::RefStoreError, index_store::IndexStore, types::*, GitService, ObjectStore, RefStore,
};
use ragfs::lock::{FilesystemPathLockProvider, LockToken, PathLockKind, PathLockProvider};
use ragfs::plugins::azureblobfs::{
    store::{Auth, AzureConfig, BlobStore},
    AzureFS,
};
use std::sync::Arc;

fn config() -> AzureConfig {
    AzureConfig {
        namespace: match std::env::var("OV_AZURE_TEST_NAMESPACE").as_deref() {
            Ok("hns") => ragfs::plugins::azureblobfs::store::Namespace::Hns,
            Err(_) | Ok("flat") => Default::default(),
            other => panic!("invalid namespace {other:?}"),
        },
        account_url: std::env::var("OV_AZURE_TEST_URL").expect("test account URL required"),
        container: std::env::var("OV_AZURE_TEST_CONTAINER").expect("test container required"),
        prefix: format!("ov-tests/{}", uuid::Uuid::new_v4()),
        auth: if std::env::var_os("OV_AZURE_TEST_SAS").is_some() {
            Auth::SasEnv {
                env: "OV_AZURE_TEST_SAS".into(),
            }
        } else if std::env::var("OV_AZURE_TEST_AUTH").as_deref() == Ok("azure_cli") {
            Auth::AzureCli {
                subscription: std::env::var("OV_AZURE_TEST_SUBSCRIPTION").ok(),
            }
        } else {
            Auth::ManagedIdentity {
                client_id: std::env::var("AZURE_CLIENT_ID").ok(),
            }
        },
    }
}
#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn stale_path_lock_owner_cannot_remove_successor() {
    let cfg = config();
    let a = FilesystemPathLockProvider::new(Arc::new(AzureFS::open(&cfg).await.unwrap()), 60.0);
    let b = FilesystemPathLockProvider::new(Arc::new(AzureFS::open(&cfg).await.unwrap()), 60.0);
    let x = LockToken::held("a", "1", PathLockKind::Exact, 1);
    let y = LockToken::held("b", "2", PathLockKind::Tree, 2);
    let p = "/.path.ovlock";
    let (l, r) = tokio::join!(a.try_create_token(p, &x), b.try_create_token(p, &y));
    assert_ne!(l.is_ok(), r.is_ok(), "{l:?} {r:?}");
    let (old, new) = if l.is_ok() { (&x, &y) } else { (&y, &x) };
    assert!(b.compare_and_write_token(p, old, new).await.unwrap());
    assert!(!a.compare_and_remove_token(p, old).await.unwrap());
    assert!(!a.compare_and_write_token(p, old, old).await.unwrap());
    assert_eq!(b.read_token(p).await.unwrap().as_ref(), Some(new));
    assert!(b.compare_and_remove_token(p, new).await.unwrap());
}
#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn competing_ref_updates_have_one_winner() {
    let cfg = config();
    let a = BlobStore::new(&cfg).unwrap();
    let b = BlobStore::new(&cfg).unwrap();
    let oid = |p: &[u8]| gix_object::compute_hash(gix_hash::Kind::Sha1, gix_object::Kind::Blob, p);
    let name = "refs/heads/main";
    for expected in [None, Some(oid(b"base"))] {
        let (l, r) = tokio::join!(
            a.cas_update("ac", name, expected, oid(b"left")),
            b.cas_update("ac", name, expected, oid(b"right"))
        );
        assert!(
            matches!(
                (&l, &r),
                (Ok(()), Err(RefStoreError::Conflict { .. }))
                    | (Err(RefStoreError::Conflict { .. }), Ok(()))
            ),
            "{l:?} {r:?}"
        );
        let winner = if l.is_ok() {
            oid(b"left")
        } else {
            oid(b"right")
        };
        assert_eq!(
            a.list("ac", "refs/heads/").await.unwrap(),
            vec![(name.into(), winner)]
        );
        a.cas_update("ac", name, Some(winner), oid(b"base"))
            .await
            .unwrap();
    }
    a.cas_update("ac", "refs/heads/team/topic", None, oid(b"topic"))
        .await
        .unwrap();
    let refs = a.list("ac", "refs").await.unwrap();
    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&("refs/heads/team/topic".into(), oid(b"topic"))));
}
#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn blob_file_operations() {
    let cfg = config();
    let fs = AzureFS::open(&cfg).await.unwrap();
    for path in ["/a", "/ab", "/a/empty"] {
        fs.mkdir(path, 0o755).await.unwrap();
    }
    let p = "/a/资料 #1%.txt";
    fs.write(p, b"abcd", 0, WriteFlag::CreateNew).await.unwrap();
    fs.write(p, b"X", 1, WriteFlag::None).await.unwrap();
    fs.write(p, b"!", 0, WriteFlag::Append).await.unwrap();
    assert_eq!(fs.read(p, 1, 2).await.unwrap(), b"Xc");
    assert!(matches!(
        fs.rename("/a", "/ab").await,
        Err(Error::AlreadyExists(_))
    ));
    assert!(fs.rename("/a", "/a/child").await.is_err());
    fs.rename("/a", "/moved").await.unwrap();
    assert!(matches!(fs.stat("/a").await, Err(Error::NotFound(_))));
    assert!(fs.stat("/moved/empty").await.unwrap().is_dir);
    fs.write("/ab/value", b"before", 0, WriteFlag::Create)
        .await
        .unwrap();
    fs.replace("/moved/资料 #1%.txt", "/ab/value")
        .await
        .unwrap();
    fs.remove_all("/moved").await.unwrap();
    assert_eq!(fs.read("/ab/value", 0, 0).await.unwrap(), b"aXcd!");
    assert!(fs.remove_all("/").await.is_err());
    assert!(fs.read("/../x", 0, 0).await.is_err());
    assert!(matches!(fs.stat(p).await, Err(Error::NotFound(_))));
}
#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn paged_listing_and_multiblock_move() {
    use futures::{stream, StreamExt, TryStreamExt};
    let fs = AzureFS::open(&config()).await.unwrap();
    fs.mkdir("/tree", 0o750).await.unwrap();
    stream::iter(0..130)
        .map(|i| {
            let fs = &fs;
            async move { fs.create(&format!("/tree/item-{i:03}")).await }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let data = vec![0x62; 9 * 1024 * 1024];
    fs.write("/tree/payload%2E.", &data, 0, WriteFlag::CreateNew)
        .await
        .unwrap();
    fs.chmod("/tree/payload%2E.", 0o600).await.unwrap();
    let page = fs
        .read_dir("/tree", Some(128), Some(3), None, None)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["item-128", "item-129", "payload%2E."]
    );
    fs.rename("/tree/payload%2E.", "/published").await.unwrap();
    assert_eq!(fs.stat("/published").await.unwrap().mode, 0o600);
    assert_eq!(fs.read("/published", 0, 0).await.unwrap(), data);
    fs.truncate("/published", 2).await.unwrap();
    assert_eq!(fs.read("/published", 0, 0).await.unwrap(), b"bb");
    fs.remove_all("/tree").await.unwrap();
}

// Fixture-only metrics: no credentials or cloud endpoints are used by this helper.
fn listing_metrics(
    cfg: &AzureConfig,
    reset: bool,
    fault: Option<(&str, &str)>,
) -> serde_json::Value {
    use std::io::{Read, Write};
    assert_eq!(std::env::var("OV_HNS_FIXTURE").as_deref(), Ok("1"));
    let mut url = azure_core::http::Url::parse(&cfg.account_url).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    url.set_path("/__fixture/metrics");
    let root = format!("/{}/{}/nodes/tree", cfg.container, cfg.prefix);
    url.query_pairs_mut().append_pair("prefix", &root);
    if let Some((kind, name)) = fault {
        url.query_pairs_mut()
            .append_pair(kind, &format!("{root}/{name}"));
    }
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    write!(
        socket,
        "{} {}?{} HTTP/1.0\r\nHost: localhost\r\n\r\n",
        if reset { "DELETE" } else { "GET" },
        url.path(),
        url.query().unwrap()
    )
    .unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.0 200"), "{headers}");
    serde_json::from_str(body).unwrap()
}

#[tokio::test]
#[ignore = "requires HNS HTTP fixture"]
async fn hns_listing_fetches_only_page_metadata_with_bounded_concurrency() {
    use futures::{stream, StreamExt, TryStreamExt};
    // Real-cloud acceptance runs the other tests without the fixture control API.
    if std::env::var("OV_HNS_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    let cfg = config();
    let fs = AzureFS::open(&cfg).await.unwrap();
    fs.mkdir("/tree", 0o750).await.unwrap();
    stream::iter(0..130)
        .map(|i| {
            let fs = &fs;
            async move { fs.create(&format!("/tree/item-{i:03}")).await }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    fs.chmod("/tree/item-128", 0o600).await.unwrap();

    listing_metrics(&cfg, true, None);
    let page = fs
        .read_dir("/tree", Some(128), Some(3), None, None)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["item-128", "item-129"]
    );
    assert_eq!(page[0].mode, 0o600);
    assert_eq!(listing_metrics(&cfg, false, None)["heads"], 2);

    listing_metrics(&cfg, true, None);
    let all = fs.read_dir("/tree", None, None, None, None).await.unwrap();
    assert_eq!(all.len(), 130);
    let metrics = listing_metrics(&cfg, false, None);
    assert_eq!(metrics["heads"], 130);
    let peak = metrics["peak"].as_u64().unwrap();
    assert!((2..=8).contains(&peak), "metadata concurrency: {metrics}");

    listing_metrics(&cfg, true, None);
    assert!(fs
        .read_dir("/tree", None, Some(0), None, None)
        .await
        .unwrap()
        .is_empty());
    assert!(fs
        .read_dir("/tree", Some(200), Some(3), None, None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(listing_metrics(&cfg, false, None)["heads"], 0);

    listing_metrics(&cfg, true, Some(("missing", "item-000")));
    let page = fs
        .read_dir("/tree", None, Some(2), None, None)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["item-001"]
    );
    listing_metrics(&cfg, true, Some(("denied", "item-000")));
    assert!(matches!(
        fs.read_dir("/tree", None, Some(1), None, None).await,
        Err(Error::PermissionDenied(_))
    ));
}

#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn listing_preserves_global_sort_and_virtual_metadata() {
    let fs = AzureFS::open(&config()).await.unwrap();
    fs.mkdir("/tree", 0o750).await.unwrap();
    for dir in ["z-dir", "A-dir"] {
        fs.mkdir(&format!("/tree/{dir}"), 0o700).await.unwrap();
    }
    for name in ["z.txt", "a.txt", "A.txt", "a%2E."] {
        fs.write(
            &format!("/tree/{name}"),
            b"payload",
            0,
            WriteFlag::CreateNew,
        )
        .await
        .unwrap();
        fs.chmod(&format!("/tree/{name}"), 0o600).await.unwrap();
    }
    // Azure timestamps have one-second resolution; make mtime order differ from name order.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    fs.write("/tree/a.txt", b"new", 0, WriteFlag::Truncate)
        .await
        .unwrap();
    let page = fs
        .read_dir("/tree", Some(1), Some(4), None, None)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["z-dir", "a%2E.", "A.txt", "a.txt"]
    );
    assert!(page[0].is_dir);
    assert_eq!(page[0].mode, 0o700);
    assert_eq!(page[1].mode, 0o600);
    assert_eq!(page[1].size, 7);
    assert_eq!(page[3].size, 3);
    let page = fs
        .read_dir(
            "/tree",
            Some(1),
            Some(3),
            Some(ListSortBy::Name),
            Some(SortOrder::Desc),
        )
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["A-dir", "z.txt", "a.txt"]
    );
    for order in [SortOrder::Asc, SortOrder::Desc] {
        let all = fs
            .read_dir("/tree", None, None, Some(ListSortBy::Mtime), Some(order))
            .await
            .unwrap();
        assert!(all[..2].iter().all(|i| i.is_dir));
        assert!(all[2..].windows(2).all(|w| if order == SortOrder::Asc {
            w[0].mod_time <= w[1].mod_time
        } else {
            w[0].mod_time >= w[1].mod_time
        }));
        let page = fs
            .read_dir(
                "/tree",
                Some(2),
                Some(2),
                Some(ListSortBy::Mtime),
                Some(order),
            )
            .await
            .unwrap();
        assert_eq!(
            page.iter().map(|i| &i.name).collect::<Vec<_>>(),
            all[2..4].iter().map(|i| &i.name).collect::<Vec<_>>()
        );
        if order == SortOrder::Desc {
            assert_eq!(page[0].name, "a.txt");
        }
    }
    fs.remove_all("/tree").await.unwrap();
}

async fn mounted_listing_pushdown(encrypted: bool) {
    use futures::{stream, StreamExt, TryStreamExt};
    use ragfs::core::{
        build_default_stack, BuilderEncryptionConfig, ConfigValue, PluginConfig, RagfsConfig,
    };
    use std::collections::HashMap;
    let cfg = config();
    let stack = build_default_stack(RagfsConfig {
        encryption: encrypted.then_some(BuilderEncryptionConfig {
            root_key: [7; 32],
            provider_type: 1,
        }),
        ..Default::default()
    })
    .await
    .unwrap();
    let auth = match &cfg.auth {
        Auth::SasEnv { env } => serde_json::json!({"type": "sas_env", "env": env}),
        Auth::AzureCli { subscription } => {
            serde_json::json!({"type": "azure_cli", "subscription": subscription})
        }
        Auth::ManagedIdentity { client_id } => {
            serde_json::json!({"type": "managed_identity", "client_id": client_id})
        }
    };
    stack.mountable.mount(PluginConfig::single_backend(
        "azureblobfs", "/local", HashMap::from([("azure_blob".into(), ConfigValue::Json(serde_json::json!({
            "account_url": cfg.account_url,
            "container": cfg.container,
            "prefix": cfg.prefix,
            "namespace": if cfg.namespace == ragfs::plugins::azureblobfs::store::Namespace::Hns { "hns" } else { "flat" },
            "auth": auth
        })))]),
    )).await.unwrap();
    let raw = AzureFS::open(&cfg).await.unwrap();
    // Seed the raw backend to include internal names; listing must never read file bodies.
    raw.mkdir("/tree", 0o750).await.unwrap();
    for dir in ["z-dir", "A-dir", ".exact.ovlock.hidden-directory"] {
        raw.mkdir(&format!("/tree/{dir}"), 0o700).await.unwrap();
    }
    let mut names: Vec<String> = (0..130).map(|i| format!("item-{i:03}")).collect();
    names.extend(
        [
            ".path.ovlock",
            ".exact.ovlock.a",
            ".sync_log.json",
            ".redirect.json",
            "backend_meta.json",
            ".keep",
            ".path.ovlock-extra",
            "资料%2E.",
            "A.txt",
            "a.txt",
        ]
        .into_iter()
        .map(String::from),
    );
    stream::iter(names)
        .map(|name| {
            let raw = &raw;
            async move {
                raw.write(
                    &format!("/tree/{name}"),
                    b"payload",
                    0,
                    WriteFlag::CreateNew,
                )
                .await?;
                raw.chmod(&format!("/tree/{name}"), 0o600).await
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    raw.write("/tree/item-000", b"latest", 0, WriteFlag::Truncate)
        .await
        .unwrap();
    let visible = |name: &str| {
        name != ".path.ovlock"
            && !name.starts_with(".exact.ovlock.")
            && name != ".sync_log.json"
            && name != ".redirect.json"
            && (!encrypted || name != "backend_meta.json")
    };
    let fixture = std::env::var("OV_HNS_FIXTURE").as_deref() == Ok("1");
    for (sort, order) in [
        (None, None),
        (Some(ListSortBy::Name), Some(SortOrder::Desc)),
        (Some(ListSortBy::Mtime), Some(SortOrder::Asc)),
        (Some(ListSortBy::Mtime), Some(SortOrder::Desc)),
    ] {
        // Independent oracle: the existing raw listing, then remove internal names.
        let expected: Vec<_> = raw
            .read_dir("/tree", None, None, sort, order)
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| visible(&entry.name))
            .collect();
        assert!(expected.iter().any(|i| i.name == ".keep"));
        assert!(expected.iter().any(|i| i.name == ".path.ovlock-extra"));
        for (offset, limit) in [
            (0, Some(3)),
            (1, Some(3)),
            (128, Some(3)),
            (0, Some(0)),
            (999, Some(3)),
            (0, None),
        ] {
            if fixture {
                listing_metrics(&cfg, true, None);
            }
            let actual = stack
                .top
                .read_dir("/local/tree", Some(offset), limit, sort, order)
                .await
                .unwrap();
            let expected_page: Vec<_> = expected
                .iter()
                .skip(offset)
                .take(limit.unwrap_or(usize::MAX))
                .collect();
            assert_eq!(actual.len(), expected_page.len());
            for (a, e) in actual.iter().zip(expected_page) {
                assert_eq!(
                    (&a.name, a.is_dir, a.mode, a.size, a.mod_time),
                    (&e.name, e.is_dir, e.mode, e.size, e.mod_time)
                );
            }
            if fixture {
                assert_eq!(
                    listing_metrics(&cfg, false, None)["heads"],
                    actual.len(),
                    "page must not hydrate hidden or out-of-page entries"
                );
            }
        }
    }
    let internal = stack
        .mountable
        .read_internal_dir("/local/tree")
        .await
        .unwrap();
    assert!(internal.iter().any(|i| i.name == ".path.ovlock"));
    assert!(internal.iter().any(|i| i.name == "backend_meta.json"));
    let root = stack
        .top
        .read_dir("/local", None, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        root.iter().any(|i| i.name == "backend_meta.json"),
        !encrypted
    );
    raw.remove_all("/tree").await.unwrap();
    stack.mountable.unmount("/local").await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn mounted_plain_listing_filters_before_pagination() {
    mounted_listing_pushdown(false).await;
}

#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn mounted_encrypted_listing_filters_before_pagination() {
    mounted_listing_pushdown(true).await;
}

#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn snapshot_commit_reopen_show_restore() {
    let cfg = config();
    let fs = Arc::new(AzureFS::open(&cfg).await.unwrap());
    let mut gc = cfg.clone();
    gc.prefix.push_str("-git");
    let store = Arc::new(BlobStore::new(&gc).unwrap());
    let svc = GitService::with_index(
        fs.clone(),
        store.clone(),
        store.clone(),
        Some(store.clone()),
    );
    let p = "/local/ac/resources/a.md";
    fs.ensure_parent_dirs(p, 0o755).await.unwrap();
    let req = |msg: &str| CommitRequest {
        account: "ac".into(),
        branch: "main".into(),
        message: msg.into(),
        paths: None,
        author_name: "tester".into(),
        author_email: "test@example.org".into(),
    };
    fs.write(p, b"this is v1", 0, WriteFlag::Create)
        .await
        .unwrap();
    let CommitResponse::Created {
        commit_oid: first, ..
    } = svc.commit(req("initial")).await.unwrap()
    else {
        panic!("expected initial commit")
    };
    assert!(store.load("ac", "main").await.unwrap().is_some());
    assert!(matches!(
        store.get_limited("ac", &first, 1).await,
        Err(ragfs::git::error::ObjectStoreError::ReadLimitExceeded { .. })
    ));
    fs.write(p, b"this is v2", 0, WriteFlag::Create)
        .await
        .unwrap();
    let CommitResponse::Created {
        commit_oid: second, ..
    } = svc.commit(req("change")).await.unwrap()
    else {
        panic!("same-size change lost")
    };
    drop(svc);
    let store = Arc::new(BlobStore::new(&gc).unwrap());
    let fresh = Arc::new(AzureFS::open(&cfg).await.unwrap());
    let svc = GitService::with_index(fresh.clone(), store.clone(), store.clone(), Some(store));
    let resp = svc
        .show(ShowRequest {
            account: "ac".into(),
            target_ref: first.to_string(),
            path: Some("resources/a.md".into()),
        })
        .await
        .unwrap();
    assert!(matches!(resp,ShowResponse::Blob{bytes,..} if bytes.as_ref()==b"this is v1"));
    let resp = svc
        .restore(RestoreRequest {
            account: "ac".into(),
            branch: "main".into(),
            project_dir: Some("resources".into()),
            source_commit: first.to_string(),
            allow_missing_source: false,
            dry_run: false,
            message: None,
            author_name: "tester".into(),
            author_email: "test@example.org".into(),
        })
        .await
        .unwrap();
    assert!(matches!(resp,RestoreResponse::Applied{parent_commit,..} if parent_commit==second));
    assert_eq!(fresh.read(p, 0, 0).await.unwrap(), b"this is v1");
}

/// Rendezvous at the publication boundary so both *commits* have built trees against the
/// same parent, irrespective of scheduler timing. Both conditional writes go to Blob.
struct CompetingRefs {
    store: BlobStore,
    barrier: Arc<tokio::sync::Barrier>,
}
#[async_trait::async_trait]
impl RefStore for CompetingRefs {
    async fn read(&self, a: &str, n: &str) -> Result<gix_hash::ObjectId, RefStoreError> {
        self.store.read(a, n).await
    }
    async fn list(
        &self,
        a: &str,
        p: &str,
    ) -> Result<Vec<(String, gix_hash::ObjectId)>, RefStoreError> {
        self.store.list(a, p).await
    }
    async fn cas_update(
        &self,
        a: &str,
        n: &str,
        old: Option<gix_hash::ObjectId>,
        new: gix_hash::ObjectId,
    ) -> Result<(), RefStoreError> {
        self.barrier.wait().await;
        self.store.cas_update(a, n, old, new).await
    }
}
#[tokio::test]
#[ignore = "requires disposable Blob container"]
async fn commits_from_same_parent_have_one_winner() {
    let cfg = config();
    let fs = Arc::new(AzureFS::open(&cfg).await.unwrap());
    let mut gc = cfg.clone();
    gc.prefix.push_str("-git");
    let store = Arc::new(BlobStore::new(&gc).unwrap());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let service = || {
        GitService::new(
            fs.clone(),
            store.clone(),
            Arc::new(CompetingRefs {
                store: BlobStore::new(&gc).unwrap(),
                barrier: barrier.clone(),
            }),
        )
    };
    let req = |n: &str| CommitRequest {
        account: "ac".into(),
        branch: "main".into(),
        message: n.into(),
        paths: Some(vec![n.into()]),
        author_name: "tester".into(),
        author_email: "tester@example.org".into(),
    };
    fs.ensure_parent_dirs("/local/ac/seed", 0o755)
        .await
        .unwrap();
    for name in ["seed", "left", "right"] {
        fs.write(
            &format!("/local/ac/{name}"),
            name.as_bytes(),
            0,
            WriteFlag::Create,
        )
        .await
        .unwrap();
    }
    GitService::new(fs.clone(), store.clone(), store.clone())
        .commit(req("seed"))
        .await
        .unwrap();
    let a = service();
    let b = service();
    let (l, r) = tokio::time::timeout(std::time::Duration::from_secs(40), async {
        tokio::join!(a.commit(req("left")), b.commit(req("right")))
    })
    .await
    .unwrap();
    let (winner, loser) = if l.is_ok() { (l, r) } else { (r, l) };
    let CommitResponse::Created { commit_oid, .. } = winner.unwrap() else {
        panic!("commit not created")
    };
    assert!(matches!(
        loser,
        Err(ragfs::git::GitError::ConcurrentCommit { .. })
    ));
    assert_eq!(
        store.read("ac", "refs/heads/main").await.unwrap(),
        commit_oid
    );
}
