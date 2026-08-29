use nucle_hardware::{
    IdtConfig, IdtProvider, IlluminaConfig, IlluminaProvider, JobHandle, NanoporeAuth,
    NanoporeConfig, NanoporeProvider, Provider, TwistConfig, TwistProvider,
};
use nucle_lang::hardware::HardwareRequest;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

const CONFIG_SIZE_LIMIT: u64 = 1024 * 1024;
const JOB_FILE_SIZE_LIMIT: u64 = 16 * 1024 * 1024;
const JOB_ENTRY_LIMIT: usize = 10_000;
const RESUME_TOKEN_SIZE_LIMIT: usize = 4096;

pub enum ConfiguredVendor {
    Twist(TwistProvider),
    Idt(IdtProvider),
    Illumina(IlluminaProvider),
    Nanopore(NanoporeProvider),
}

impl ConfiguredVendor {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Twist(_) => "twist",
            Self::Idt(_) => "idt",
            Self::Illumina(_) => "illumina",
            Self::Nanopore(_) => "nanopore",
        }
    }

    pub fn provider(&self) -> &dyn Provider {
        match self {
            Self::Twist(provider) => provider,
            Self::Idt(provider) => provider,
            Self::Illumina(provider) => provider,
            Self::Nanopore(provider) => provider,
        }
    }

    pub fn validate_batch(&self, batch: &[HardwareRequest]) -> Result<(), String> {
        self.provider().validate_batch(batch)
    }

    pub fn resume(&self, token: &str) -> Result<Box<dyn JobHandle>, String> {
        match self {
            Self::Twist(provider) => provider.resume(token),
            Self::Idt(provider) => provider.resume(token),
            Self::Illumina(provider) => provider.resume(token),
            Self::Nanopore(provider) => provider.resume(token),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
enum VendorConfigFile {
    Twist(TwistFileConfig),
    Idt(IdtFileConfig),
    Illumina(IlluminaFileConfig),
    Nanopore(NanoporeFileConfig),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TwistFileConfig {
    base_url: String,
    order_path: String,
    status_path_template: String,
    email_env: String,
    api_token_env: String,
    #[serde(default = "default_twist_auth_header")]
    auth_header_name: String,
    #[serde(default = "default_twist_email_header")]
    email_header_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdtFileConfig {
    client_id_env: String,
    client_secret_env: String,
    username_env: String,
    password_env: String,
    #[serde(default = "default_idt_token_url")]
    token_url: String,
    api_base_url: String,
    order_path: String,
    status_path_template: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IlluminaFileConfig {
    #[serde(default = "default_illumina_base_url")]
    base_url: String,
    access_token_env: String,
    launch_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NanoporeFileConfig {
    host: String,
    manager_port: u16,
    ca_certificate_path: PathBuf,
    protocol_identifier: String,
    auth: NanoporeFileAuth,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum NanoporeFileAuth {
    DeveloperApiToken {
        token_env: String,
    },
    ClientCertificate {
        cert_chain_path: PathBuf,
        private_key_path: PathBuf,
    },
}

fn default_twist_auth_header() -> String {
    "Authorization".to_string()
}

fn default_twist_email_header() -> String {
    "X-Twist-Email".to_string()
}

fn default_idt_token_url() -> String {
    "https://www.idtdna.com/Identityserver/connect/token".to_string()
}

fn default_illumina_base_url() -> String {
    "https://api.basespace.illumina.com/v2".to_string()
}

pub fn load_vendor_config(
    path: &Path,
    expected_provider: &str,
) -> Result<ConfiguredVendor, String> {
    let metadata = fs::metadata(path)
        .map_err(|e| format!("failed to read vendor config '{}': {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "vendor config '{}' is not a regular file",
            path.display()
        ));
    }
    if metadata.len() > CONFIG_SIZE_LIMIT {
        return Err(format!(
            "vendor config '{}' exceeds the 1 MiB limit",
            path.display()
        ));
    }
    let bytes = fs::read(path)
        .map_err(|e| format!("failed to read vendor config '{}': {e}", path.display()))?;
    let parsed: VendorConfigFile = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid vendor config '{}': {e}", path.display()))?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let provider = match parsed {
        VendorConfigFile::Twist(config) => {
            validate_base_url(&config.base_url, "base_url")?;
            validate_path(&config.order_path, "order_path")?;
            validate_status_template(&config.status_path_template)?;
            validate_header_name(&config.auth_header_name, "auth_header_name")?;
            validate_header_name(&config.email_header_name, "email_header_name")?;
            let email = load_secret(&config.email_env, "email_env")?;
            let api_token = load_secret(&config.api_token_env, "api_token_env")?;
            validate_header_value(&email, "Twist email")?;
            validate_header_value(&api_token, "Twist API token")?;
            let mut vendor = TwistConfig::new(
                config.base_url,
                config.order_path,
                config.status_path_template,
                email,
                api_token,
            );
            vendor.auth_header_name = config.auth_header_name;
            vendor.email_header_name = config.email_header_name;
            ConfiguredVendor::Twist(TwistProvider::new(vendor))
        }
        VendorConfigFile::Idt(config) => {
            validate_endpoint_url(&config.token_url, "token_url")?;
            validate_base_url(&config.api_base_url, "api_base_url")?;
            validate_path(&config.order_path, "order_path")?;
            validate_status_template(&config.status_path_template)?;
            let mut vendor = IdtConfig::new(
                load_secret(&config.client_id_env, "client_id_env")?,
                load_secret(&config.client_secret_env, "client_secret_env")?,
                load_secret(&config.username_env, "username_env")?,
                load_secret(&config.password_env, "password_env")?,
                config.api_base_url,
                config.order_path,
                config.status_path_template,
            );
            vendor.token_url = config.token_url;
            ConfiguredVendor::Idt(IdtProvider::new(vendor))
        }
        VendorConfigFile::Illumina(config) => {
            validate_base_url(&config.base_url, "base_url")?;
            validate_path(&config.launch_path, "launch_path")?;
            let access_token = load_secret(&config.access_token_env, "access_token_env")?;
            validate_header_value(&access_token, "Illumina access token")?;
            let mut vendor = IlluminaConfig::new(access_token, config.launch_path);
            vendor.base_url = config.base_url;
            ConfiguredVendor::Illumina(IlluminaProvider::new(vendor))
        }
        VendorConfigFile::Nanopore(config) => {
            validate_nanopore_host(&config.host)?;
            if config.manager_port == 0 {
                return Err("manager_port must be greater than zero".to_string());
            }
            validate_nonempty_text(&config.protocol_identifier, "protocol_identifier")?;
            let ca_certificate_pem = read_pem(
                base_dir,
                &config.ca_certificate_path,
                "ca_certificate_path",
                "CERTIFICATE",
            )?;
            let auth = match config.auth {
                NanoporeFileAuth::DeveloperApiToken { token_env } => {
                    let token = load_secret(&token_env, "auth.token_env")?;
                    validate_grpc_metadata_value(&token, "Nanopore developer API token")?;
                    NanoporeAuth::DeveloperApiToken(token)
                }
                NanoporeFileAuth::ClientCertificate {
                    cert_chain_path,
                    private_key_path,
                } => {
                    let cert_chain_pem = read_pem(
                        base_dir,
                        &cert_chain_path,
                        "auth.cert_chain_path",
                        "CERTIFICATE",
                    )?;
                    let key_pem = read_private_key(base_dir, &private_key_path)?;
                    NanoporeAuth::ClientCertificate {
                        cert_chain_pem,
                        key_pem,
                    }
                }
            };
            let vendor = NanoporeConfig {
                host: config.host,
                manager_port: config.manager_port,
                ca_certificate_pem,
                auth,
                protocol_identifier: config.protocol_identifier,
            };
            ConfiguredVendor::Nanopore(NanoporeProvider::new(vendor)?)
        }
    };

    if provider.name() != expected_provider {
        return Err(format!(
            "vendor config declares provider '{}' but --provider is '{}'",
            provider.name(),
            expected_provider
        ));
    }
    Ok(provider)
}

fn load_secret(env_name: &str, field: &str) -> Result<String, String> {
    if env_name.is_empty() || env_name.contains('=') || env_name.chars().any(char::is_control) {
        return Err(format!("{field} must name a valid environment variable"));
    }
    match std::env::var(env_name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!(
            "credential environment variable '{env_name}' is empty"
        )),
        Err(std::env::VarError::NotPresent) => Err(format!(
            "credential environment variable '{env_name}' is not set"
        )),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!(
            "credential environment variable '{env_name}' is not valid Unicode"
        )),
    }
}

fn validate_endpoint_url(value: &str, field: &str) -> Result<(), String> {
    validate_nonempty_text(value, field)?;
    let secure = value.starts_with("https://");
    let loopback = value.starts_with("http://127.0.0.1:")
        || value.starts_with("http://localhost:")
        || value.starts_with("http://[::1]:");
    if !secure && !loopback {
        return Err(format!(
            "{field} must use HTTPS (plain HTTP is allowed only for an explicit loopback test server)"
        ));
    }
    let separator = value.find("://").unwrap_or(value.len());
    if value.get(separator + 3..).unwrap_or("").is_empty()
        || value.contains('@')
        || value.chars().any(char::is_whitespace)
    {
        return Err(format!("{field} is not a valid credential-safe URL"));
    }
    Ok(())
}

fn validate_base_url(value: &str, field: &str) -> Result<(), String> {
    validate_endpoint_url(value, field)?;
    if value.ends_with('/') || value.contains('?') || value.contains('#') {
        return Err(format!(
            "{field} must not end in '/' or contain a query/fragment"
        ));
    }
    Ok(())
}

fn validate_path(value: &str, field: &str) -> Result<(), String> {
    validate_nonempty_text(value, field)?;
    if !value.starts_with('/')
        || value.starts_with("//")
        || value.contains("://")
        || value.contains('#')
    {
        return Err(format!(
            "{field} must be a single-host absolute path beginning with '/'"
        ));
    }
    Ok(())
}

fn validate_status_template(value: &str) -> Result<(), String> {
    validate_path(value, "status_path_template")?;
    if value.matches("{id}").count() != 1 {
        return Err("status_path_template must contain exactly one '{id}' placeholder".to_string());
    }
    Ok(())
}

fn validate_header_name(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
    {
        return Err(format!("{field} is not a valid HTTP header name"));
    }
    Ok(())
}

fn validate_header_value(value: &str, field: &str) -> Result<(), String> {
    if value
        .chars()
        .any(|character| character == '\r' || character == '\n')
    {
        return Err(format!("{field} contains a forbidden newline"));
    }
    Ok(())
}

fn validate_grpc_metadata_value(value: &str, field: &str) -> Result<(), String> {
    if !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err(format!("{field} must contain visible ASCII only"));
    }
    Ok(())
}

fn validate_nanopore_host(value: &str) -> Result<(), String> {
    validate_nonempty_text(value, "host")?;
    if value.contains("://")
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_whitespace)
    {
        return Err(
            "host must be a hostname or IP address without a scheme, port, or path".to_string(),
        );
    }
    Ok(())
}

fn validate_nonempty_text(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(format!(
            "{field} must be non-empty and contain no control characters"
        ));
    }
    Ok(())
}

fn resolve_config_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn read_pem(base_dir: &Path, path: &Path, field: &str, label: &str) -> Result<Vec<u8>, String> {
    let resolved = resolve_config_path(base_dir, path);
    let bytes = fs::read(&resolved)
        .map_err(|e| format!("failed to read {field} '{}': {e}", resolved.display()))?;
    if bytes.is_empty() || bytes.len() > CONFIG_SIZE_LIMIT as usize {
        return Err(format!(
            "{field} must be a non-empty PEM file no larger than 1 MiB"
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| format!("{field} '{}' is not UTF-8 PEM", resolved.display()))?;
    if !text.contains(&format!("-----BEGIN {label}-----"))
        || !text.contains(&format!("-----END {label}-----"))
    {
        return Err(format!(
            "{field} '{}' is not a {label} PEM file",
            resolved.display()
        ));
    }
    Ok(bytes)
}

fn read_private_key(base_dir: &Path, path: &Path) -> Result<Vec<u8>, String> {
    let resolved = resolve_config_path(base_dir, path);
    let bytes = fs::read(&resolved).map_err(|e| {
        format!(
            "failed to read auth.private_key_path '{}': {e}",
            resolved.display()
        )
    })?;
    if bytes.is_empty() || bytes.len() > CONFIG_SIZE_LIMIT as usize {
        return Err(
            "auth.private_key_path must be a non-empty PEM file no larger than 1 MiB".to_string(),
        );
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        format!(
            "auth.private_key_path '{}' is not UTF-8 PEM",
            resolved.display()
        )
    })?;
    if !text.contains("-----BEGIN PRIVATE KEY-----")
        && !text.contains("-----BEGIN RSA PRIVATE KEY-----")
        && !text.contains("-----BEGIN EC PRIVATE KEY-----")
    {
        return Err(format!(
            "auth.private_key_path '{}' is not a private-key PEM file",
            resolved.display()
        ));
    }
    Ok(bytes)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JobHeader {
    format: String,
    version: u8,
    provider: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobEntry {
    pub source: String,
    pub resume_token: String,
    pub requests_count: usize,
    pub effectful_requests_confirmed: bool,
}

pub struct JobWriter {
    writer: BufWriter<File>,
}

impl JobWriter {
    pub fn create(path: &Path, provider: &str) -> Result<Self, String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create job-file directory '{}': {e}",
                    parent.display()
                )
            })?;
        }
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("refusing to overwrite job file '{}': {e}", path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(
            &mut writer,
            &JobHeader {
                format: "nucle-hardware-job".to_string(),
                version: 1,
                provider: provider.to_string(),
            },
        )
        .map_err(|e| format!("failed to serialize job header: {e}"))?;
        writer
            .write_all(b"\n")
            .map_err(|e| format!("failed to write job header: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("failed to flush job header: {e}"))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|e| format!("failed to sync job header: {e}"))?;
        Ok(Self { writer })
    }

    pub fn append(&mut self, entry: &JobEntry) -> Result<(), String> {
        serde_json::to_writer(&mut self.writer, entry)
            .map_err(|e| format!("failed to serialize submitted job: {e}"))?;
        self.writer
            .write_all(b"\n")
            .map_err(|e| format!("failed to write submitted job: {e}"))?;
        self.writer
            .flush()
            .map_err(|e| format!("failed to flush submitted job: {e}"))?;
        self.writer
            .get_ref()
            .sync_data()
            .map_err(|e| format!("failed to sync submitted job: {e}"))?;
        Ok(())
    }
}

pub fn load_job_file(path: &Path) -> Result<(String, Vec<JobEntry>), String> {
    let metadata = fs::metadata(path)
        .map_err(|e| format!("failed to read job file '{}': {e}", path.display()))?;
    if !metadata.is_file() || metadata.len() > JOB_FILE_SIZE_LIMIT {
        return Err(format!(
            "job file '{}' must be a regular file no larger than 16 MiB",
            path.display()
        ));
    }
    let file = File::open(path)
        .map_err(|e| format!("failed to read job file '{}': {e}", path.display()))?;
    let mut lines = BufReader::new(file).lines();
    let header_line = lines
        .next()
        .ok_or_else(|| format!("job file '{}' is empty", path.display()))?
        .map_err(|e| format!("failed to read job file '{}': {e}", path.display()))?;
    let header: JobHeader = serde_json::from_str(&header_line)
        .map_err(|e| format!("invalid job-file header '{}': {e}", path.display()))?;
    if header.format != "nucle-hardware-job"
        || header.version != 1
        || !is_vendor_provider(&header.provider)
    {
        return Err(format!(
            "job file '{}' has an unsupported format, version, or provider",
            path.display()
        ));
    }

    let mut entries = Vec::new();
    for (index, line) in lines.enumerate() {
        if index >= JOB_ENTRY_LIMIT {
            return Err(format!(
                "job file '{}' exceeds the {}-record limit",
                path.display(),
                JOB_ENTRY_LIMIT
            ));
        }
        let line =
            line.map_err(|e| format!("failed to read job file '{}': {e}", path.display()))?;
        if line.trim().is_empty() {
            return Err(format!(
                "job file '{}' contains an empty record at line {}",
                path.display(),
                index + 2
            ));
        }
        let entry: JobEntry = serde_json::from_str(&line).map_err(|e| {
            format!(
                "invalid job record at '{}:{}': {e}",
                path.display(),
                index + 2
            )
        })?;
        if entry.source.is_empty()
            || entry.source.chars().any(char::is_control)
            || entry.requests_count == 0
            || !entry.effectful_requests_confirmed
        {
            return Err(format!(
                "job record at '{}:{}' contains invalid request metadata",
                path.display(),
                index + 2
            ));
        }
        if entry.resume_token.is_empty() || entry.resume_token.len() > RESUME_TOKEN_SIZE_LIMIT {
            return Err(format!(
                "job record at '{}:{}' has an invalid resume token size",
                path.display(),
                index + 2
            ));
        }
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(format!(
            "job file '{}' contains no submitted jobs",
            path.display()
        ));
    }
    Ok((header.provider, entries))
}

pub fn is_vendor_provider(provider: &str) -> bool {
    matches!(provider, "twist" | "idt" | "illumina" | "nanopore")
}
