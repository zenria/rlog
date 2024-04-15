use std::{
    error::Error,
    fs::{create_dir_all, File},
    io::{Read, Write},
    path::Path,
};

use anyhow::Context;
use clap::{Parser, Subcommand};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use time::OffsetDateTime;

#[derive(Parser)]
struct Opts {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// generate CA, server and client certificates
    Cert {
        /// Where to write certificate, also where to read CA certificates from.
        #[arg(short, long, default_value = "./ca")]
        output_dir: String,
        #[command(subcommand)]
        command: CertificateCommand,
    },
    /// Minimal quickwit index schema
    PrintQuickwitSchema,
}

#[derive(Subcommand)]
enum CertificateCommand {
    /// Generate self signed certificates to be used as certification authority.
    GenerateCA {
        #[arg(long)]
        country: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        locality: Option<String>,
        #[arg(long)]
        organisation: Option<String>,
        #[arg(long)]
        organisation_unit: Option<String>,
        /// When the certificate expires? in human time format (eg. "1M" = 1 month, "1y" = 1 year)
        #[arg(long, default_value = "10y")]
        expires_in: String,
        /// mandatory common name for this CA
        common_name: String,
    },
    /// Generate server certificate. output_dir must contain ca-priv-key.pem and ca.pem (output of generate-ca command)
    GenerateServer {
        /// When the certificate expires? in human time format (eg. "1M" = 1 month, "1y" = 1 year)
        #[arg(long, default_value = "1y")]
        expires_in: String,
        #[arg(long)]
        alt_dns_hostname: Vec<String>,
        /// DNS hostname (will be put in the common name of the certificate)
        hostname: String,
    },
    /// Generate client certificate.
    ///
    /// If the client certificate has already been generated (a private key for that client exists),
    /// it will re-generate a new certificate using the same private key.
    GenerateClient {
        /// When the certificate expires? in human time format (eg. "1M" = 1 month, "1y" = 1 year)
        #[arg(long, default_value = "1y")]
        expires_in: String,
        /// Force the generation of a private key even if the key for the client exists.
        #[arg(short, long)]
        new_private_key: bool,
        /// Name of the client (common name)
        client_name: String,
    },
}

impl CertificateCommand {
    fn generate(&self, output_dir: String) -> Result<(), Box<dyn Error>> {
        match self {
            CertificateCommand::GenerateCA {
                country,
                state,
                locality,
                organisation,
                organisation_unit,
                common_name,
                expires_in,
            } => {
                create_dir_all(&output_dir)
                    .with_context(|| format!("Unable to create output directory {output_dir}"))?;

                let mut params = CertificateParams::default();
                params.distinguished_name = DistinguishedName::new();
                params
                    .distinguished_name
                    .push(DnType::CommonName, common_name);
                params.not_before = OffsetDateTime::now_utc();
                params.not_after = params.not_before
                    + humantime::parse_duration(&expires_in)
                        .context("Unable to parse expires-in argument")?;

                if let Some(country) = country {
                    params.distinguished_name.push(DnType::CountryName, country);
                }
                if let Some(state) = state {
                    params
                        .distinguished_name
                        .push(DnType::StateOrProvinceName, state);
                }
                if let Some(locality) = locality {
                    params
                        .distinguished_name
                        .push(DnType::LocalityName, locality);
                }
                if let Some(organisation) = organisation {
                    params
                        .distinguished_name
                        .push(DnType::OrganizationName, organisation);
                }
                if let Some(organisation_unit) = organisation_unit {
                    params
                        .distinguished_name
                        .push(DnType::OrganizationalUnitName, organisation_unit);
                }
                let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)?;
                let ca_cert = params.self_signed(&key_pair)?;

                {
                    let pem_ca_key = key_pair.serialize_pem();
                    let key_file_name = ca_key_filename(&output_dir);
                    File::create(&key_file_name)
                        .with_context(|| format!("Unable to open file {key_file_name}"))?
                        .write_all(pem_ca_key.as_bytes())?;
                    println!("CA private key written to {key_file_name}: \n{pem_ca_key}\n");
                }
                {
                    let pem_ca_cert = ca_cert.pem();
                    let cert_file_name = ca_cert_filename(&output_dir);
                    File::create(&cert_file_name)
                        .with_context(|| format!("Unable to open file {cert_file_name}"))?
                        .write_all(pem_ca_cert.as_bytes())?;
                    println!("CA certificate written to {cert_file_name}: \n{pem_ca_cert}\n");
                }
            }
            CertificateCommand::GenerateServer {
                expires_in,
                alt_dns_hostname,
                hostname,
            } => {
                let (ca_certificate_params, ca_key_pair) =
                    parse_ca_certificate(&output_dir).context("Unable to load CA certificates")?;

                // Why I'm forced to do this?
                let ca_certificate = ca_certificate_params.self_signed(&ca_key_pair)?;

                let mut subject_alt_name = Vec::new();
                subject_alt_name.push(hostname.clone());
                subject_alt_name.extend(alt_dns_hostname.iter().cloned());

                let mut params = CertificateParams::new(subject_alt_name)?;

                params.distinguished_name = DistinguishedName::new();
                params.distinguished_name.push(DnType::CommonName, hostname);
                params.not_before = OffsetDateTime::now_utc();
                params.not_after = params.not_before
                    + humantime::parse_duration(&expires_in)
                        .context("Unable to parse expires-in argument")?;

                let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)?;
                let cert = params.signed_by(&key_pair, &ca_certificate, &ca_key_pair)?;
                {
                    let key = key_pair.serialize_pem();
                    let key_file_name = format!("{output_dir}/{hostname}.priv-key.pem");
                    File::create(&key_file_name)
                        .with_context(|| format!("Unable to open file {key_file_name}"))?
                        .write_all(key.as_bytes())?;
                    println!("{hostname} server private key written to {key_file_name}: \n{key}\n");
                }
                {
                    let pem = cert.pem();
                    let cert_file_name = format!("{output_dir}/{hostname}.pem");
                    File::create(&cert_file_name)
                        .with_context(|| format!("Unable to open file {cert_file_name}"))?
                        .write_all(pem.as_bytes())?;
                    println!(
                        "{hostname} server certificate written to {cert_file_name}: \n{pem}\n"
                    );
                }
            }
            CertificateCommand::GenerateClient {
                expires_in,
                client_name,
                new_private_key,
            } => {
                let (ca_certificate_params, ca_key_pair) =
                    parse_ca_certificate(&output_dir).context("Unable to load CA certificates")?;

                // Why I'm forced to do this?
                let ca_certificate = ca_certificate_params.self_signed(&ca_key_pair)?;

                let mut params = CertificateParams::default();
                params.distinguished_name = DistinguishedName::new();
                params
                    .distinguished_name
                    .push(DnType::CommonName, client_name);
                params.not_before = OffsetDateTime::now_utc();
                params.not_after = params.not_before
                    + humantime::parse_duration(&expires_in)
                        .context("Unable to parse expires-in argument")?;

                let key_file_name = format!("{output_dir}/{client_name}.priv-key.pem");
                let (key_pair, private_key_not_generated) = if *new_private_key {
                    (
                        KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)?,
                        false,
                    )
                } else {
                    // ignore errors: if any error occurs reading private key PEM
                    // (inexistent file, file corrupt), a new private key will be generated
                    if let Some(key_pair) = load_keypair(&key_file_name).ok() {
                        (key_pair, true)
                    } else {
                        (
                            KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)?,
                            false,
                        )
                    }
                };

                let cert = params.signed_by(&key_pair, &ca_certificate, &ca_key_pair)?;
                if private_key_not_generated {
                    println!(
                        "{client_name} existing client private key used for certificate generation"
                    );
                } else {
                    let key = key_pair.serialize_pem();
                    File::create(&key_file_name)
                        .with_context(|| format!("Unable to open file {key_file_name}"))?
                        .write_all(key.as_bytes())?;
                    println!(
                        "{client_name} client private key written to {key_file_name}: \n{key}\n"
                    );
                }
                {
                    let pem = cert.pem();
                    let cert_file_name = format!("{output_dir}/{client_name}.pem");
                    File::create(&cert_file_name)
                        .with_context(|| format!("Unable to open file {cert_file_name}"))?
                        .write_all(pem.as_bytes())?;
                    println!(
                        "{client_name} client certificate written to {cert_file_name}: \n{pem}\n"
                    );
                }
            }
        }
        Ok(())
    }
}

fn ca_key_filename(output_dir: &str) -> String {
    format!("{output_dir}/ca.priv-key.pem")
}

fn ca_cert_filename(output_dir: &str) -> String {
    format!("{output_dir}/ca.pem")
}

fn load_keypair<P: AsRef<Path>>(path: P) -> anyhow::Result<KeyPair> {
    let path = path.as_ref();
    Ok(KeyPair::from_pem(
        &std::fs::read_to_string(path).with_context(|| {
            format!("Unable to open private key from {}", path.to_string_lossy())
        })?,
    )
    .context("Unable to parse private key PEM")?)
}

fn parse_ca_certificate(output_dir: &str) -> anyhow::Result<(CertificateParams, KeyPair)> {
    let ca_key_file_name = ca_key_filename(&output_dir);
    let mut ca_priv_pem = String::new();
    File::open(&ca_key_file_name)
        .with_context(|| format!("Unable to open CA private key from {ca_key_file_name}"))?
        .read_to_string(&mut ca_priv_pem)?;
    let ca_keypair = load_keypair(&ca_key_file_name).context("Unable to load CA private key")?;

    let ca_file_name = ca_cert_filename(&output_dir);
    let mut ca_pem = String::new();
    File::open(&ca_file_name)
        .with_context(|| format!("Unable to open CA cetificate from {ca_file_name}"))?
        .read_to_string(&mut ca_pem)?;
    let params = CertificateParams::from_ca_cert_pem(&ca_pem)?;

    Ok((params, ca_keypair))
}

fn main() -> Result<(), Box<dyn Error>> {
    let opts = Opts::parse();
    match opts.command {
        Command::PrintQuickwitSchema => println!("{}", include_str!("schema.yaml")),
        Command::Cert {
            output_dir,
            command,
        } => {
            command.generate(output_dir)?;
        }
    }
    Ok(())
}
