//! Strict certificate profiles; inspect public DER and verify key/signature identity.
use super::*;
use x509_parser::extensions::ParsedExtension;
use x509_parser::prelude::parse_x509_certificate;

fn certificate(pem_text: &str) -> Result<Vec<u8>, String> {
    pem::parse(pem_text)
        .map(|value| value.into_contents())
        .map_err(|_| "invalid certificate PEM".into())
}

fn supported_extensions(
    cert: &x509_parser::certificate::X509Certificate<'_>,
) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for extension in cert.extensions() {
        let oid = extension.oid.to_id_string();
        if !seen.insert(oid.clone())
            || !matches!(
                oid.as_str(),
                "2.5.29.14" | "2.5.29.15" | "2.5.29.17" | "2.5.29.19" | "2.5.29.35" | "2.5.29.37"
            )
        {
            return Err("certificate contains unsupported or duplicate extensions; explicit expert migration required".into());
        }
    }
    Ok(())
}

pub(super) fn same_leaf_identity(old: &str, new: &str) -> Result<(), String> {
    let old_der = certificate(old)?;
    let new_der = certificate(new)?;
    let (_, old) = parse_x509_certificate(&old_der).map_err(|_| "invalid old leaf")?;
    let (_, new) = parse_x509_certificate(&new_der).map_err(|_| "invalid candidate leaf")?;
    let sans = |cert: &x509_parser::certificate::X509Certificate<'_>| {
        cert.extensions()
            .iter()
            .find(|extension| extension.oid.to_id_string() == "2.5.29.17")
            .map(|extension| (extension.critical, extension.value.to_vec()))
    };
    if old.subject().as_raw() != new.subject().as_raw()
        || old.public_key().raw != new.public_key().raw
        || sans(&old) != sans(&new)
        || old.validity() != new.validity()
    {
        return Err(
            "upgrade would change leaf subject, public key, SANs or validity; refusing candidate"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn same_root_identity(old: &str, new: &str) -> Result<(), String> {
    let old_der = certificate(old)?;
    let new_der = certificate(new)?;
    let (_, old) = parse_x509_certificate(&old_der).map_err(|_| "invalid old CA")?;
    let (_, new) = parse_x509_certificate(&new_der).map_err(|_| "invalid candidate CA")?;
    let ski = |cert: &x509_parser::certificate::X509Certificate<'_>| {
        cert.extensions().iter().find_map(|extension| {
            if let ParsedExtension::SubjectKeyIdentifier(value) = extension.parsed_extension() {
                Some(value.0.to_vec())
            } else {
                None
            }
        })
    };
    if old.subject().as_raw() != new.subject().as_raw()
        || old.public_key().raw != new.public_key().raw
        || ski(&old) != ski(&new)
        || old.validity() != new.validity()
    {
        return Err(
            "upgrade would change root subject, public key, SKI or validity; refusing candidate"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn ca_pair(
    cert_pem: &str,
    key_pem: &str,
    strict: bool,
) -> Result<CertificateParams, String> {
    let der = certificate(cert_pem)?;
    let (_, cert) = parse_x509_certificate(&der).map_err(|_| "invalid CA certificate DER")?;
    supported_extensions(&cert)?;
    let key = KeyPair::from_pem(key_pem).map_err(|_| "invalid CA private key")?;
    if cert.public_key().raw != key.public_key_der() {
        return Err("CA certificate/private key mismatch; refusing replacement".into());
    }
    if cert.subject() != cert.issuer() || cert.verify_signature(None).is_err() {
        return Err("CA must be a valid self-signed root".into());
    }
    if !cert.validity().is_valid() {
        return Err("CA certificate is expired or not yet valid; explicit renewal required".into());
    }
    let basic = cert
        .basic_constraints()
        .map_err(|_| "invalid CA basic constraints")?
        .ok_or("CA basic constraints missing")?;
    if !basic.critical || !basic.value.ca {
        return Err("CA requires critical CA=true basic constraints".into());
    }
    let ski = cert.extensions().iter().find_map(|extension| {
        if let ParsedExtension::SubjectKeyIdentifier(value) = extension.parsed_extension() {
            Some(value.0)
        } else {
            None
        }
    });
    if ski.is_none() {
        return Err("CA subject key identifier missing; cannot preserve root identity".into());
    }
    if strict {
        let usage = cert.key_usage().map_err(|_| "invalid CA key usage")?;
        if !usage.is_some_and(|value| {
            value.critical && value.value.key_cert_sign() && value.value.crl_sign()
        }) {
            return Err("CA profile requires explicit upgrade (critical keyCertSign/cRLSign missing); run `vzctl certs ca upgrade --output <new-private-directory>`, then review and promote the candidate with trust rollout; never use --force".into());
        }
    }
    CertificateParams::from_ca_cert_pem(cert_pem).map_err(|_| "cannot parse CA parameters".into())
}

pub(super) fn leaf_pair(
    cert_pem: &str,
    key_pem: &str,
    ca_pem: &str,
    strict: bool,
) -> Result<CertificateParams, String> {
    let der = certificate(cert_pem)?;
    let root_der = certificate(ca_pem)?;
    let (_, cert) = parse_x509_certificate(&der).map_err(|_| "invalid leaf certificate DER")?;
    supported_extensions(&cert)?;
    let (_, root) =
        parse_x509_certificate(&root_der).map_err(|_| "invalid root certificate DER")?;
    let key = KeyPair::from_pem(key_pem).map_err(|_| "invalid leaf private key")?;
    if cert.public_key().raw != key.public_key_der() {
        return Err("leaf certificate/private key mismatch; refusing replacement".into());
    }
    if cert.issuer() != root.subject() || cert.verify_signature(Some(root.public_key())).is_err() {
        return Err("leaf is not signed by the current root; refusing replacement".into());
    }
    if strict {
        if !cert.validity().is_valid() {
            return Err(
                "leaf certificate is expired or not yet valid; explicit renewal required".into(),
            );
        }
        let ski = root.extensions().iter().find_map(|extension| {
            if let ParsedExtension::SubjectKeyIdentifier(value) = extension.parsed_extension() {
                Some(value.0)
            } else {
                None
            }
        });
        let aki = cert.extensions().iter().find_map(|extension| {
            if let ParsedExtension::AuthorityKeyIdentifier(value) = extension.parsed_extension() {
                value.key_identifier.as_ref().map(|value| value.0)
            } else {
                None
            }
        });
        if aki.is_none() || aki != ski {
            return Err("leaf authority key identifier missing or mismatched".into());
        }
        if !cert
            .key_usage()
            .map_err(|_| "invalid leaf key usage")?
            .is_some_and(|value| value.value.digital_signature() && !value.value.key_cert_sign())
            || !cert
                .extended_key_usage()
                .map_err(|_| "invalid leaf extended key usage")?
                .is_some_and(|value| value.value.server_auth)
        {
            return Err("leaf requires digitalSignature and serverAuth usage".into());
        }
    }
    CertificateParams::from_ca_cert_pem(cert_pem).map_err(|_| "cannot parse leaf parameters".into())
}
