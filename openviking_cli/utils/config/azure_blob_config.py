"""Configuration shared by the native Azure content and Git stores."""

from typing import Annotated, Literal
from urllib.parse import urlsplit

from pydantic import BaseModel, ConfigDict, Field, model_validator


class ManagedIdentity(BaseModel):
    model_config = ConfigDict(extra="forbid")
    type: Literal["managed_identity"] = "managed_identity"
    client_id: str | None = None


class AzureCli(BaseModel):
    model_config = ConfigDict(extra="forbid")
    type: Literal["azure_cli"] = "azure_cli"
    subscription: str | None = None


class EmulatorSas(BaseModel):
    model_config = ConfigDict(extra="forbid")
    type: Literal["sas_env"]
    env: str = Field(pattern=r"^[A-Za-z_][A-Za-z0-9_]*$")


class AzureBlobConfig(BaseModel):
    """A pre-created Blob container and an exclusive application prefix."""

    model_config = ConfigDict(extra="forbid")
    namespace: Literal["flat", "hns"] = "flat"
    account_url: str
    container: str = Field(pattern=r"^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$")
    prefix: str = "content"
    auth: Annotated[ManagedIdentity | AzureCli | EmulatorSas, Field(discriminator="type")] = Field(
        default_factory=ManagedIdentity
    )

    def container_location(self) -> tuple:
        """Normalize default ports and account endpoints for overlap checks."""
        url = urlsplit(self.account_url)
        return (
            url.hostname,
            url.port or (443 if url.scheme == "https" else 80),
            url.path.rstrip("/"),
            self.container,
        )

    @model_validator(mode="after")
    def validate_location(self):
        url = urlsplit(self.account_url)
        emulator = url.hostname in {"127.0.0.1", "::1", "localhost"}
        if not url.hostname or url.username or url.password or url.query or url.fragment:
            raise ValueError("account_url must be an unsigned account endpoint")
        if isinstance(self.auth, EmulatorSas):
            if not emulator or url.scheme not in {"http", "https"}:
                raise ValueError("sas_env is reserved for a loopback Blob emulator")
        elif url.scheme != "https" or url.path.strip("/"):
            raise ValueError("Azure identity requires an HTTPS account endpoint")
        if "\\" in self.prefix or any(ord(ch) < 32 or ord(ch) == 127 for ch in self.prefix):
            raise ValueError("invalid Azure Blob prefix")
        if "--" in self.container or any(p in {"", ".", ".."} for p in self.prefix.split("/")):
            raise ValueError("invalid Azure container or prefix")
        return self
