"""Azure configuration and native mount acceptance, using an isolated target prefix per test."""

import os
import uuid
from urllib.parse import urlencode
from urllib.request import Request, urlopen

import pytest
from pydantic import ValidationError

from openviking.storage.errors import LockAcquisitionError
from openviking.utils.agfs_utils import RagfsBindingConfig, create_agfs_client, mount_agfs_backend
from openviking_cli.utils.config.azure_blob_config import AzureBlobConfig
from openviking_cli.utils.config.open_viking_config import OpenVikingConfig


def sections(tmp_path, location, git):
    raw = {
        "storage": {
            "workspace": str(tmp_path),
            "agfs": {"backend": "azure_blob", "azure_blob": location},
        },
        "git": git,
    }
    raw["embedding"] = {
        "dense": {
            "provider": "openai",
            "model": "text-embedding-3-small",
            "api_key": "unused-test-value",
            "dimension": 1536,
        }
    }
    config = OpenVikingConfig.model_validate(raw)
    return config.storage, config.git


def test_azure_config_inherits_identity_and_rejects_overlapping_prefixes(tmp_path):
    loc = {
        "account_url": "https://contoso.blob.core.windows.net",
        "container": "openviking",
        "prefix": "prod/content",
    }
    storage, git = sections(tmp_path, loc, {"azure_blob": {"prefix": "prod/git"}})
    assert git.backend == "azure_blob"
    assert storage.agfs.azure_blob.namespace == git.azure_blob.namespace == "flat"
    hns, hns_git = sections(tmp_path, {**loc, "namespace": "hns"}, {})
    assert hns.agfs.azure_blob.namespace == hns_git.azure_blob.namespace == "hns"
    assert git.azure_blob.container == storage.agfs.azure_blob.container
    assert git.azure_blob.auth.type == "managed_identity"
    cli, cli_git = sections(tmp_path, {**loc, "auth": {"type": "azure_cli"}}, {})
    assert cli.agfs.azure_blob.auth.type == cli_git.azure_blob.auth.type == "azure_cli"
    _, scoped_git = sections(
        tmp_path, {**loc, "auth": {"type": "azure_cli", "subscription": "test-subscription"}}, {}
    )
    assert scoped_git.azure_blob.auth.subscription == "test-subscription"
    with pytest.raises(ValueError, match="same namespace"):
        sections(tmp_path, loc, {"azure_blob": {"namespace": "hns", "container": "other"}})
    for prefix in ("prod", "prod/content", "prod/content/git"):
        with pytest.raises(ValueError, match="disjoint"):
            sections(tmp_path, loc, {"azure_blob": {"prefix": prefix}})
    for override in (
        {"prefix": "../priv"},
        {"namespace": "automatic"},
        {"account_url": loc["account_url"] + "?sig=invalid"},
        {"auth": {"type": "sas_env", "env": "TEST_SAS"}},
    ):
        with pytest.raises(ValidationError):
            AzureBlobConfig.model_validate({**loc, **override})


@pytest.mark.skipif(not os.getenv("OV_AZURE_TEST_URL"), reason="requires disposable Blob container")
@pytest.mark.parametrize("encrypted", [False, True])
def test_mount_from_openviking_and_publish(tmp_path, encrypted):
    base = f"ov-tests/{uuid.uuid4()}"
    loc = {
        "namespace": os.getenv("OV_AZURE_TEST_NAMESPACE", "flat"),
        "account_url": os.environ["OV_AZURE_TEST_URL"],
        "container": os.environ["OV_AZURE_TEST_CONTAINER"],
        "prefix": base + "/content",
    }
    if "OV_AZURE_TEST_SAS" in os.environ:
        loc["auth"] = {"type": "sas_env", "env": "OV_AZURE_TEST_SAS"}
    elif os.getenv("OV_AZURE_TEST_AUTH") == "azure_cli":
        loc["auth"] = {
            "type": "azure_cli",
            "subscription": os.getenv("OV_AZURE_TEST_SUBSCRIPTION"),
        }
    elif os.getenv("AZURE_CLIENT_ID"):
        loc["auth"] = {"type": "managed_identity", "client_id": os.environ["AZURE_CLIENT_ID"]}
    storage, git = sections(
        tmp_path, loc, {"enabled": True, "azure_blob": {"prefix": base + "/git"}}
    )
    binding = RagfsBindingConfig(
        agfs=storage.agfs,
        root_key=os.urandom(32) if encrypted else None,
        provider_type=1 if encrypted else None,
    )
    client = create_agfs_client(binding, git_config=git)
    mount_agfs_backend(client, binding)
    assert client.health()["git_backend"] == "azure_blob"
    p = "/local/ac/resources/draft/a.md"
    published = "/local/ac/resources/published/a.md"
    ctx = {"account_id": "ac"}
    client.ensure_parent_dirs(p, ctx=ctx)
    lease = client.pathlock_acquire_tree(ctx, "/local/ac/resources")
    # Exercise Python -> stats -> pathlock -> mount -> optional encryption -> Azure.
    # The live lock marker must be filtered before applying the visible-page offset.
    metrics_url = None
    if os.getenv("OV_HNS_FIXTURE") == "1":
        metrics_url = (
            loc["account_url"]
            + "/__fixture/metrics?"
            + urlencode({"prefix": f"/{loc['container']}/{loc['prefix']}/nodes/ac/resources"})
        )
        with urlopen(Request(metrics_url, method="DELETE"), timeout=10) as response:
            assert response.status == 200
    assert [
        entry["name"] for entry in client.ls("/local/ac/resources", ctx=ctx, offset=0, limit=1)
    ] == ["draft"]
    assert client.ls("/local/ac/resources", ctx=ctx, offset=1, limit=1) == []
    if metrics_url:
        import json

        with urlopen(metrics_url, timeout=10) as response:
            assert json.load(response)["heads"] == 1
    with pytest.raises(LockAcquisitionError):
        client.pathlock_acquire_exact(ctx, p)
    assert client.pathlock_refresh(ctx, lease) == "refreshed"
    client.pathlock_release(ctx, lease)
    client.pathlock_release(ctx, client.pathlock_acquire_exact(ctx, p))

    snapshots = []
    for path, data in ((p, b"first"), (published, b"second")):
        if path == published:
            client.mv("/local/ac/resources/draft", "/local/ac/resources/published", ctx=ctx)
            assert client.read(published, ctx=ctx) == b"first"
        client.write(path, data, ctx=ctx)
        assert client.read(path, ctx=ctx) == data
        snapshot = client.git_commit(
            account="ac",
            branch="main",
            message="capture",
            paths=["resources/draft/a.md"]
            if path == p
            else ["resources/draft/a.md", "resources/published/a.md"],
            author_name="tester",
            author_email="tester@example.org",
        )
        assert snapshot["result"] == "created"
        snapshots.append(snapshot["commit_oid"])
    historical = client.git_show(
        account="ac", target_ref=snapshots[0], path="resources/draft/a.md"
    )["bytes"]
    assert historical == b"first"
    restored = client.git_restore(
        account="ac",
        branch="main",
        project_dir="resources",
        source_commit=snapshots[0],
        author_name="tester",
        author_email="tester@example.org",
        ctx=ctx,
    )
    assert restored["parent_commit"] == snapshots[1]
    assert client.read(p, ctx=ctx) == b"first"
    if encrypted:
        # Account-scoped decryption must reject the same object with another account key.
        with pytest.raises(Exception, match="(?i)decrypt|authentication|crypto"):
            client.read(p, ctx={"account_id": "other"})
