use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ServiceKind {
    Ingress,
    Oidc,
    Postgres,
    S3,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    pub kind: ServiceKind,
    pub via: Option<String>,
    pub endpoint: Option<String>,
    pub advertise_endpoint: Option<String>,
    pub admin_user: Option<String>,
    pub password_file: Option<String>,
    pub access_key_file: Option<String>,
    pub secret_key_file: Option<String>,
    /// S3 IAM provisioning is provider-specific; v1 supports MinIO explicitly.
    pub implementation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClaimConfig {
    pub kind: ServiceKind,
    pub provider: Option<String>,
    pub database: Option<String>,
    pub bucket: Option<String>,
    #[serde(rename = "clientID", alias = "clientId")]
    pub client_id: Option<String>,
    #[serde(default, rename = "redirectURIs", alias = "redirectUris")]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub host: Option<String>,
    pub backend: Option<Backend>,
    #[serde(default)]
    pub consumers: Vec<Consumer>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum Consumer {
    Vm(VmConsumer),
    Container(ContainerConsumer),
    Kubernetes(KubernetesConsumer),
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct VmConsumer {
    pub vm: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContainerConsumer {
    pub container: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct KubernetesConsumer {
    pub kubernetes: KubernetesTarget,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct KubernetesTarget {
    pub cluster: String,
    pub namespace: String,
    pub secret: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum Backend {
    Vm(VmBackend),
    Kubernetes(KubernetesBackend),
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct VmBackend {
    pub vm: String,
    pub port: u16,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct KubernetesBackend {
    pub kubernetes: KubernetesService,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct KubernetesService {
    pub cluster: String,
    pub namespace: String,
    pub service: String,
    pub port: u16,
}

pub(crate) type PublicBinding = BTreeMap<String, String>;
