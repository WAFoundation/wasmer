use std::sync::Arc;

use anyhow::{Context, Error};
use http::{HeaderMap, Method};
use semver::Version;
use url::Url;
use webc::metadata::Manifest;

use crate::{
    http::{HttpClient, HttpRequest, USER_AGENT},
    runtime::resolver::{
        DistributionInfo, PackageInfo, PackageSpecifier, PackageSummary, Source, WebcHash,
    },
};

/// A [`Source`] which will resolve dependencies by pinging a Wasmer-like GraphQL
/// endpoint.
#[derive(Debug, Clone)]
pub struct WapmSource {
    registry_endpoint: Url,
    client: Arc<dyn HttpClient + Send + Sync>,
}

impl WapmSource {
    pub const WASMER_DEV_ENDPOINT: &str = "https://registry.wasmer.wtf/graphql";
    pub const WASMER_PROD_ENDPOINT: &str = "https://registry.wasmer.io/graphql";

    pub fn new(registry_endpoint: Url, client: Arc<dyn HttpClient + Send + Sync>) -> Self {
        WapmSource {
            registry_endpoint,
            client,
        }
    }
}

#[async_trait::async_trait]
impl Source for WapmSource {
    #[tracing::instrument(level = "debug", skip_all, fields(%package))]
    async fn query(&self, package: &PackageSpecifier) -> Result<Vec<PackageSummary>, Error> {
        let (full_name, version_constraint) = match package {
            PackageSpecifier::Registry { full_name, version } => (full_name, version),
            _ => return Ok(Vec::new()),
        };
        #[derive(serde::Serialize)]
        struct Body {
            query: String,
        }

        let body = Body {
            query: WASMER_WEBC_QUERY_ALL.replace("$NAME", full_name),
        };
        let body = serde_json::to_string(&body)?;
        tracing::trace!(%body, "Sending GraphQL query");

        let request = HttpRequest {
            url: self.registry_endpoint.clone(),
            method: Method::POST,
            body: Some(body.into_bytes()),
            headers: headers(),
            options: Default::default(),
        };

        let response = self.client.request(request).await?;

        if !response.is_ok() {
            let url = &self.registry_endpoint;
            let status = response.status;
            anyhow::bail!("\"{url}\" replied with {status}");
        }

        let body = response.body.unwrap_or_default();
        tracing::trace!(
            body=?String::from_utf8_lossy(&body),
            "Received a response from GraphQL",
        );

        let response: WapmWebQuery =
            serde_json::from_slice(&body).context("Unable to deserialize the response")?;

        let mut summaries = Vec::new();

        let versions = match response.data.get_package {
            Some(WapmWebQueryGetPackage { versions }) => versions,
            None => return Ok(Vec::new()),
        };

        for pkg_version in versions {
            tracing::trace!(?pkg_version, "checking package version");

            let version = Version::parse(&pkg_version.version)?;
            if version_constraint.matches(&version) {
                match decode_summary(pkg_version) {
                    Ok(summary) => summaries.push(summary),
                    Err(e) => {
                        tracing::debug!(
                            version=%version,
                            error=&*e,
                            "Skipping version because its metadata couldn't be parsed"
                        );
                    }
                }
            }
        }

        Ok(summaries)
    }
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("User-Agent", USER_AGENT.parse().unwrap());
    headers
}

fn decode_summary(pkg_version: WapmWebQueryGetPackageVersion) -> Result<PackageSummary, Error> {
    let WapmWebQueryGetPackageVersion {
        manifest,
        distribution:
            WapmWebQueryGetPackageVersionDistribution {
                pirita_download_url,
                pirita_sha256_hash,
            },
        ..
    } = pkg_version;

    let manifest = manifest.context("missing Manifest")?;
    let hash = pirita_sha256_hash.context("missing sha256")?;
    let url = pirita_download_url.context("missing download url")?;

    let manifest: Manifest = serde_json::from_slice(manifest.as_bytes())
        .context("Unable to deserialize the manifest")?;

    let mut webc_sha256 = [0_u8; 32];
    hex::decode_to_slice(&hash, &mut webc_sha256)?;
    let webc_sha256 = WebcHash::from_bytes(webc_sha256);

    Ok(PackageSummary {
        pkg: PackageInfo::from_manifest(&manifest)?,
        dist: DistributionInfo {
            webc: url.parse().context("Unable to parse the download URL")?,
            webc_sha256,
        },
    })
}

#[allow(dead_code)]
pub const WASMER_WEBC_QUERY_ALL: &str = r#"{
    getPackage(name: "$NAME") {
        versions {
        version
        piritaManifest
        distribution {
            piritaDownloadUrl
            piritaSha256Hash
        }
        }
    }
}"#;

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct WapmWebQuery {
    #[serde(rename = "data")]
    pub data: WapmWebQueryData,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct WapmWebQueryData {
    #[serde(rename = "getPackage")]
    pub get_package: Option<WapmWebQueryGetPackage>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct WapmWebQueryGetPackage {
    pub versions: Vec<WapmWebQueryGetPackageVersion>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct WapmWebQueryGetPackageVersion {
    pub version: String,
    /// A JSON string containing a [`Manifest`] definition.
    #[serde(rename = "piritaManifest")]
    pub manifest: Option<String>,
    pub distribution: WapmWebQueryGetPackageVersionDistribution,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct WapmWebQueryGetPackageVersionDistribution {
    #[serde(rename = "piritaDownloadUrl")]
    pub pirita_download_url: Option<String>,
    #[serde(rename = "piritaSha256Hash")]
    pub pirita_sha256_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use http::{HeaderMap, StatusCode};

    use crate::{
        http::HttpResponse,
        runtime::resolver::inputs::{DistributionInfo, FileSystemMapping, PackageInfo},
    };

    use super::*;

    // You can check the response with:
    // curl https://registry.wasmer.io/graphql \
    //      -H "Content-Type: application/json" \
    //      -X POST \
    //      -d '@wasmer_pack_cli_request.json' > wasmer_pack_cli_response.json
    const WASMER_PACK_CLI_REQUEST: &[u8] = br#"{"query": "{\n    getPackage(name: \"wasmer/wasmer-pack-cli\") {\n        versions {\n        version\n        piritaManifest\n        distribution {\n            piritaDownloadUrl\n            piritaSha256Hash\n        }\n        }\n    }\n}"}"#;
    const WASMER_PACK_CLI_RESPONSE: &[u8] = br#"{"data":{"getPackage":{"versions":[{"version":"0.7.0","piritaManifest":"{\"atoms\": {\"wasmer-pack\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:FesCIAS6URjrIAAyy4G5u5HjJjGQBLGmnafjHPHRvqo=\"}}, \"package\": {\"wapm\": {\"name\": \"wasmer/wasmer-pack-cli\", \"readme\": {\"path\": \"/home/consulting/Documents/wasmer/wasmer-pack/crates/cli/../../README.md\", \"volume\": \"metadata\"}, \"license\": \"MIT\", \"version\": \"0.7.0\", \"homepage\": \"https://wasmer.io/\", \"repository\": \"https://github.com/wasmerio/wasmer-pack\", \"description\": \"A code generator that lets you treat WebAssembly modules like native dependencies.\"}}, \"commands\": {\"wasmer-pack\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"wasmer-pack\", \"package\": \"wasmer/wasmer-pack-cli\", \"main_args\": null}}}}, \"entrypoint\": \"wasmer-pack\"}","distribution":{"piritaDownloadUrl":"https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.7.0-0e384e88-ab70-11ed-b0ed-b22ba48456e7.webc","piritaSha256Hash":"d085869201aa602673f70abbd5e14e5a6936216fa93314c5b103cda3da56e29e"}},{"version":"0.6.0","piritaManifest":"{\"atoms\": {\"wasmer-pack\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:CzzhNaav3gjBkCJECGbk7e+qAKurWbcIAzQvEqsr2Co=\"}}, \"package\": {\"wapm\": {\"name\": \"wasmer/wasmer-pack-cli\", \"readme\": {\"path\": \"/home/consulting/Documents/wasmer/wasmer-pack/crates/cli/../../README.md\", \"volume\": \"metadata\"}, \"license\": \"MIT\", \"version\": \"0.6.0\", \"homepage\": \"https://wasmer.io/\", \"repository\": \"https://github.com/wasmerio/wasmer-pack\", \"description\": \"A code generator that lets you treat WebAssembly modules like native dependencies.\"}}, \"commands\": {\"wasmer-pack\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"wasmer-pack\", \"package\": \"wasmer/wasmer-pack-cli\", \"main_args\": null}}}}, \"entrypoint\": \"wasmer-pack\"}","distribution":{"piritaDownloadUrl":"https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.6.0-654a2ed8-875f-11ed-90e2-c6aeb50490de.webc","piritaSha256Hash":"7e1add1640d0037ff6a726cd7e14ea36159ec2db8cb6debd0e42fa2739bea52b"}},{"version":"0.5.3","piritaManifest":"{\"atoms\": {\"wasmer-pack\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:qdiJVfpi4icJXdR7Y5US/pJ4PjqbAq9PkU+obMZIMlE=\"}}, \"package\": {\"wapm\": {\"name\": \"wasmer/wasmer-pack-cli\", \"readme\": {\"path\": \"/home/runner/work/wasmer-pack/wasmer-pack/crates/cli/../../README.md\", \"volume\": \"metadata\"}, \"license\": \"MIT\", \"version\": \"0.5.3\", \"homepage\": \"https://wasmer.io/\", \"repository\": \"https://github.com/wasmerio/wasmer-pack\", \"description\": \"A code generator that lets you treat WebAssembly modules like native dependencies.\"}}, \"commands\": {\"wasmer-pack\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"wasmer-pack\", \"package\": \"wasmer/wasmer-pack-cli\", \"main_args\": null}}}}, \"entrypoint\": \"wasmer-pack\"}","distribution":{"piritaDownloadUrl":"https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.5.3-4a2b9764-728c-11ed-9fe4-86bf77232c64.webc","piritaSha256Hash":"44fdcdde23d34175887243d7c375e4e4a7e6e2cd1ae063ebffbede4d1f68f14a"}},{"version":"0.5.2","piritaManifest":"{\"atoms\": {\"wasmer-pack\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:xiwrUFAo+cU1xW/IE6MVseiyjNGHtXooRlkYKiOKzQc=\"}}, \"package\": {\"wapm\": {\"name\": \"wasmer/wasmer-pack-cli\", \"readme\": {\"path\": \"/home/consulting/Documents/wasmer/wasmer-pack/crates/cli/../../README.md\", \"volume\": \"metadata\"}, \"license\": \"MIT\", \"version\": \"0.5.2\", \"homepage\": \"https://wasmer.io/\", \"repository\": \"https://github.com/wasmerio/wasmer-pack\", \"description\": \"A code generator that lets you treat WebAssembly modules like native dependencies.\"}}, \"commands\": {\"wasmer-pack\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"wasmer-pack\", \"package\": \"wasmer/wasmer-pack-cli\", \"main_args\": null}}}}, \"entrypoint\": \"wasmer-pack\"}","distribution":{"piritaDownloadUrl":"https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.5.2.webc","piritaSha256Hash":"d1dbc8168c3a2491a7158017a9c88df9e0c15bed88ebcd6d9d756e4b03adde95"}},{"version":"0.5.1","piritaManifest":"{\"atoms\": {\"wasmer-pack\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:TliPwutfkFvRite/3/k3OpLqvV0EBKGwyp3L5UjCuEI=\"}}, \"package\": {\"wapm\": {\"name\": \"wasmer/wasmer-pack-cli\", \"readme\": {\"path\": \"/home/runner/work/wasmer-pack/wasmer-pack/crates/cli/../../README.md\", \"volume\": \"metadata\"}, \"license\": \"MIT\", \"version\": \"0.5.1\", \"homepage\": \"https://wasmer.io/\", \"repository\": \"https://github.com/wasmerio/wasmer-pack\", \"description\": \"A code generator that lets you treat WebAssembly modules like native dependencies.\"}}, \"commands\": {\"wasmer-pack\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"wasmer-pack\", \"package\": \"wasmer/wasmer-pack-cli\", \"main_args\": null}}}}, \"entrypoint\": \"wasmer-pack\"}","distribution":{"piritaDownloadUrl":"https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.5.1.webc","piritaSha256Hash":"c42924619660e2befd69b5c72729388985dcdcbf912d51a00015237fec3e1ade"}},{"version":"0.5.0","piritaManifest":"{\"atoms\": {\"wasmer-pack\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:6UD7NS4KtyNYa3TcnKOvd+kd3LxBCw+JQ8UWRpMXeC0=\"}}, \"package\": {\"wapm\": {\"name\": \"wasmer/wasmer-pack-cli\", \"readme\": {\"path\": \"README.md\", \"volume\": \"metadata\"}, \"license\": \"MIT\", \"version\": \"0.5.0\", \"homepage\": \"https://wasmer.io/\", \"repository\": \"https://github.com/wasmerio/wasmer-pack\", \"description\": \"A code generator that lets you treat WebAssembly modules like native dependencies.\"}}, \"commands\": {\"wasmer-pack\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"wasmer-pack\", \"package\": \"wasmer/wasmer-pack-cli\", \"main_args\": null}}}}, \"entrypoint\": \"wasmer-pack\"}","distribution":{"piritaDownloadUrl":"https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.5.0.webc","piritaSha256Hash":"d30ca468372faa96469163d2d1546dd34be9505c680677e6ab86a528a268e5f5"}},{"version":"0.5.0-rc.1","piritaManifest":"{\"atoms\": {\"wasmer-pack\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:ThybHIc2elJEcDdQiq5ffT1TVaNs70+WAqoKw4Tkh3E=\"}}, \"package\": {\"wapm\": {\"name\": \"wasmer/wasmer-pack-cli\", \"readme\": {\"path\": \"README.md\", \"volume\": \"metadata\"}, \"license\": \"MIT\", \"version\": \"0.5.0-rc.1\", \"homepage\": \"https://wasmer.io/\", \"repository\": \"https://github.com/wasmerio/wasmer-pack\", \"description\": \"A code generator that lets you treat WebAssembly modules like native dependencies.\"}}, \"commands\": {\"wasmer-pack\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"wasmer-pack\", \"package\": \"wasmer/wasmer-pack-cli\", \"main_args\": null}}}}, \"entrypoint\": \"wasmer-pack\"}","distribution":{"piritaDownloadUrl":"https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.5.0-rc.1.webc","piritaSha256Hash":"0cd5d6e4c33c92c52784afed3a60c056953104d719717948d4663ff2521fe2bb"}}]}}}"#;

    #[derive(Debug)]
    struct DummyClient {
        requests: Mutex<Vec<HttpRequest>>,
        responses: Mutex<Vec<HttpResponse>>,
    }

    impl DummyClient {
        fn new(responses: Vec<HttpResponse>) -> Self {
            DummyClient {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            }
        }

        fn take_requests(&self) -> Vec<HttpRequest> {
            std::mem::take(&mut *self.requests.lock().unwrap())
        }
    }

    impl HttpClient for DummyClient {
        fn request(
            &self,
            request: HttpRequest,
        ) -> futures::future::BoxFuture<'_, Result<HttpResponse, anyhow::Error>> {
            self.requests.lock().unwrap().push(request);
            let response = self.responses.lock().unwrap().remove(0);
            Box::pin(async { Ok(response) })
        }
    }

    #[tokio::test]
    async fn run_known_query() {
        let response = HttpResponse {
            body: Some(WASMER_PACK_CLI_RESPONSE.to_vec()),
            redirected: false,
            status: StatusCode::OK,
            headers: HeaderMap::new(),
        };
        let client = Arc::new(DummyClient::new(vec![response]));
        let registry_endpoint = WapmSource::WASMER_PROD_ENDPOINT.parse().unwrap();
        let request = PackageSpecifier::Registry {
            full_name: "wasmer/wasmer-pack-cli".to_string(),
            version: "^0.6".parse().unwrap(),
        };
        let source = WapmSource::new(registry_endpoint, client.clone());

        let summaries = source.query(&request).await.unwrap();

        assert_eq!(
            summaries,
            [PackageSummary {
                pkg: PackageInfo {
                    name: "wasmer/wasmer-pack-cli".to_string(),
                    version: Version::new(0, 6, 0),
                    dependencies: Vec::new(),
                    commands: vec![
                        crate::runtime::resolver::Command {
                            name: "wasmer-pack".to_string(),
                        },
                    ],
                    entrypoint: Some("wasmer-pack".to_string()),
                    filesystem: vec![FileSystemMapping {
                        volume_name: "atom".to_string(),
                        mount_path: "/".to_string(),
                        original_path: "/".to_string(),
                        dependency_name: None,
                    }],
                },
                dist: DistributionInfo {
                    webc: "https://registry-cdn.wasmer.io/packages/wasmer/wasmer-pack-cli/wasmer-pack-cli-0.6.0-654a2ed8-875f-11ed-90e2-c6aeb50490de.webc".parse().unwrap(),
                    webc_sha256: WebcHash::from_bytes([
                        126,
                        26,
                        221,
                        22,
                        64,
                        208,
                        3,
                        127,
                        246,
                        167,
                        38,
                        205,
                        126,
                        20,
                        234,
                        54,
                        21,
                        158,
                        194,
                        219,
                        140,
                        182,
                        222,
                        189,
                        14,
                        66,
                        250,
                        39,
                        57,
                        190,
                        165,
                        43,
                    ]),
                }
            }]
        );
        let requests = client.take_requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, http::Method::POST);
        assert_eq!(request.url.as_str(), WapmSource::WASMER_PROD_ENDPOINT);
        assert_eq!(request.headers.len(), 2);
        assert_eq!(request.headers["User-Agent"], USER_AGENT);
        assert_eq!(request.headers["Content-Type"], "application/json");
        let body: serde_json::Value =
            serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
        let expected_body: serde_json::Value =
            serde_json::from_slice(WASMER_PACK_CLI_REQUEST).unwrap();
        assert_eq!(body, expected_body);
    }

    /// For the full context, see #3946 on GitHub or the original conversation
    /// [on
    /// Slack](https://wasmerio.slack.com/archives/C03MX4KL6KH/p1685706988500919).
    #[tokio::test]
    async fn skip_package_versions_with_missing_fields() {
        let body = serde_json::json! {
            {
                "data": {
                    "getPackage": {
                        "versions": [
                            {
                                "version": "0.2.0",
                                "piritaManifest": "{\"atoms\": {\"cowsay\": {\"kind\": \"https://webc.org/kind/wasm\", \"signature\": \"sha256:DPmhiSNXCg5261eTUi3BIvAc/aJttGj+nD+bGhQkVQo=\"}}, \"package\": {\"wapm\": {\"name\": \"cowsay\", \"readme\": {\"path\": \"README.md\", \"volume\": \"metadata\"}, \"version\": \"0.2.0\", \"repository\": \"https://github.com/wapm-packages/cowsay\", \"description\": \"cowsay is a program that generates ASCII pictures of a cow with a message\"}}, \"commands\": {\"cowsay\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"cowsay\", \"package\": null, \"main_args\": null}}}, \"cowthink\": {\"runner\": \"https://webc.org/runner/wasi/command@unstable_\", \"annotations\": {\"wasi\": {\"atom\": \"cowsay\", \"package\": null, \"main_args\": null}}}}}",
                                "distribution": {
                                    "piritaDownloadUrl": "https://storage.googleapis.com/wapm-registry-prod/packages/_/cowsay/cowsay-0.2.0.webc",
                                    "piritaSha256Hash": "9586938a0a89219dafe4ae97a901c56d4b3e2a9941520d1309ae880c9a1868c9"
                                }
                            },
                            {
                                "version": "0.1.3",
                                "piritaManifest": null,
                                "distribution": {
                                    "piritaDownloadUrl": "https://example.com/",
                                    "piritaSha256Hash": "1234asdf"
                                }
                            },
                            {
                                "version": "0.1.2",
                                "piritaManifest": "{}",
                                "distribution": {
                                    "piritaDownloadUrl": null,
                                    "piritaSha256Hash": "1234asdf"
                                }
                            },
                            {
                                "version": "0.1.3",
                                "piritaManifest": "{}",
                                "distribution": {
                                    "piritaDownloadUrl": "https://example.com/",
                                    "piritaSha256Hash":null
                                }
                            }
                        ]
                    }
                }
            }

        };
        let response = HttpResponse {
            body: Some(serde_json::to_vec(&body).unwrap()),
            redirected: false,
            status: StatusCode::OK,
            headers: HeaderMap::new(),
        };
        let client = Arc::new(DummyClient::new(vec![response]));
        let registry_endpoint = WapmSource::WASMER_PROD_ENDPOINT.parse().unwrap();
        let request = PackageSpecifier::Registry {
            full_name: "_/cowsay".to_string(),
            version: semver::VersionReq::STAR,
        };
        let source = WapmSource::new(registry_endpoint, client.clone());

        let summaries = source.query(&request).await.unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].pkg.version.to_string(), "0.2.0");
    }
}
