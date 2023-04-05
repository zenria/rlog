use std::{
    error::Error,
    fs::{create_dir_all, File},
    io::{Read, Write},
};

use anyhow::Context;
use clap::{Parser, Subcommand};
use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, KeyPair};

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
        /// mandatory common name for this CA
        common_name: String,
    },
    /// Generate server certificate. output_dir must contain ca-priv-key.pem and ca.pem (output of generate-ca command)
    GenerateServer {
        #[arg(long)]
        alt_dns_hostname: Vec<String>,
        /// DNS hostname (will be put in the common name of the certificate)
        hostname: String,
    },
    GenerateClient {
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
            } => {
                create_dir_all(&output_dir)
                    .with_context(|| format!("Unable to create output directory {output_dir}"))?;

                let mut params = CertificateParams::default();
                params.distinguished_name = DistinguishedName::new();
                params
                    .distinguished_name
                    .push(DnType::CommonName, common_name);
                params.alg = &rcgen::PKCS_ECDSA_P384_SHA384;

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

                let ca_cert = Certificate::from_params(params)?;

                {
                    let pem_ca_key = ca_cert.serialize_private_key_pem();
                    let key_file_name = ca_key_filename(&output_dir);
                    File::create(&key_file_name)
                        .with_context(|| format!("Unable to open file {key_file_name}"))?
                        .write_all(pem_ca_key.as_bytes())?;
                    println!("CA private key written to {key_file_name}: \n{pem_ca_key}\n");
                }
                {
                    let pem_ca_cert = ca_cert.serialize_pem()?;
                    let cert_file_name = ca_cert_filename(&output_dir);
                    File::create(&cert_file_name)
                        .with_context(|| format!("Unable to open file {cert_file_name}"))?
                        .write_all(pem_ca_cert.as_bytes())?;
                    println!("CA certificate written to {cert_file_name}: \n{pem_ca_cert}\n");
                }
            }
            CertificateCommand::GenerateServer {
                alt_dns_hostname,
                hostname,
            } => {
                let ca_certificate =
                    parse_ca_certificate(&output_dir).context("Unable to load CA certificates")?;

                let mut subject_alt_name = Vec::new();
                subject_alt_name.push(hostname.clone());
                subject_alt_name.extend(alt_dns_hostname.iter().cloned());

                let mut params = CertificateParams::new(subject_alt_name);
                params.distinguished_name = DistinguishedName::new();
                params.distinguished_name.push(DnType::CommonName, hostname);
                params.alg = &rcgen::PKCS_ECDSA_P384_SHA384;

                let cert = Certificate::from_params(params)?;
                {
                    let key = cert.serialize_private_key_pem();
                    let key_file_name = format!("{output_dir}/{hostname}.priv-key.pem");
                    File::create(&key_file_name)
                        .with_context(|| format!("Unable to open file {key_file_name}"))?
                        .write_all(key.as_bytes())?;
                    println!("{hostname} server private key written to {key_file_name}: \n{key}\n");
                }
                {
                    let pem = cert.serialize_pem_with_signer(&ca_certificate)?;
                    let cert_file_name = format!("{output_dir}/{hostname}.pem");
                    File::create(&cert_file_name)
                        .with_context(|| format!("Unable to open file {cert_file_name}"))?
                        .write_all(pem.as_bytes())?;
                    println!(
                        "{hostname} server certificate written to {cert_file_name}: \n{pem}\n"
                    );
                }
            }
            CertificateCommand::GenerateClient { client_name } => {
                let ca_certificate =
                    parse_ca_certificate(&output_dir).context("Unable to load CA certificates")?;

                let mut params = CertificateParams::default();
                params.distinguished_name = DistinguishedName::new();
                params
                    .distinguished_name
                    .push(DnType::CommonName, client_name);
                params.alg = &rcgen::PKCS_ECDSA_P384_SHA384;

                let cert = Certificate::from_params(params)?;
                {
                    let key = cert.serialize_private_key_pem();
                    let key_file_name = format!("{output_dir}/{client_name}.priv-key.pem");
                    File::create(&key_file_name)
                        .with_context(|| format!("Unable to open file {key_file_name}"))?
                        .write_all(key.as_bytes())?;
                    println!(
                        "{client_name} client private key written to {key_file_name}: \n{key}\n"
                    );
                }
                {
                    let pem = cert.serialize_pem_with_signer(&ca_certificate)?;
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

fn parse_ca_certificate(output_dir: &str) -> anyhow::Result<Certificate> {
    let ca_key_file_name = ca_key_filename(&output_dir);
    let mut ca_priv_pem = String::new();
    File::open(&ca_key_file_name)
        .with_context(|| format!("Unable to open CA private key from {ca_key_file_name}"))?
        .read_to_string(&mut ca_priv_pem)?;
    let ca_keypair =
        KeyPair::from_pem(&ca_priv_pem).context("Unable to parse CA private key PEM")?;

    let ca_file_name = ca_cert_filename(&output_dir);
    let mut ca_pem = String::new();
    File::open(&ca_file_name)
        .with_context(|| format!("Unable to open CA cetificate from {ca_file_name}"))?
        .read_to_string(&mut ca_pem)?;
    let params = CertificateParams::from_ca_cert_pem(&ca_pem, ca_keypair)?;
    Ok(Certificate::from_params(params)?)
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
