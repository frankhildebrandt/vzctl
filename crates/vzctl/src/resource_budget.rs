//! Advisory capacity budget for a stack. This is not a reservation or a free-RAM estimate.
use crate::config::Environment;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct BudgetReport {
    pub(crate) host_memory_mib: Option<u64>,
    pub(crate) requested_memory_mib: u64,
    pub(crate) reserve_mib: u64,
    pub(crate) available_for_vms_mib: Option<u64>,
    pub(crate) warnings: Vec<String>,
}

pub(crate) fn assess(environment: &Environment) -> BudgetReport {
    assess_with_host_memory(environment, host_memory_mib())
}

pub(crate) fn assess_with_host_memory(
    environment: &Environment,
    host: Option<u64>,
) -> BudgetReport {
    let mut warnings = Vec::new();
    let requested = environment.spec.vms.values().fold(0u64, |sum, vm| {
        let memory = match vm.memory.as_deref() {
            Some(value) => match crate::parse_memory_mib(value) {
                Ok(memory) => memory,
                Err(_) => {
                    warnings.push("RAM-Budget unvollständig: ungültige VM-Speicherangabe.".into());
                    0
                }
            },
            None => 1024,
        };
        sum.saturating_add(memory)
    });
    // Leave at least 2 GiB or 20% for macOS and host applications.
    let reserve = host
        .map(|total| 2048.max(total.div_ceil(5)))
        .unwrap_or(2048);
    let available = host.map(|total| total.saturating_sub(reserve));
    match available {
        Some(available) if requested > available => warnings.push(format!(
            "Stack fordert {requested} MiB RAM; nach {reserve} MiB Hostreserve bleiben {available} MiB der Gesamtkapazität. Andere Stacks und laufende Apps sind nicht eingerechnet."
        )),
        None => warnings.push("Host-RAM unbekannt; kein Kapazitätsvergleich möglich.".into()),
        _ => {}
    }
    BudgetReport {
        host_memory_mib: host,
        requested_memory_mib: requested,
        reserve_mib: reserve,
        available_for_vms_mib: available,
        warnings,
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn host_memory_mib() -> Option<u64> {
    let mut bytes = 0u64;
    let mut size = std::mem::size_of::<u64>();
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && size == std::mem::size_of::<u64>()).then_some(bytes / 1024 / 1024)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn host_memory_mib() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn environment() -> Environment {
        crate::config::validate_source(include_str!("../tests/fixtures/validate/valid-full.yaml"))
            .unwrap()
    }
    #[test]
    fn budget_warns_without_changing_configuration() {
        let mut env = environment();
        for vm in env.spec.vms.values_mut() {
            vm.memory = Some("4GiB".into());
        }
        let before = serde_json::to_value(&env).unwrap();
        let report = assess_with_host_memory(&env, Some(8192));
        assert_eq!(
            report.requested_memory_mib,
            env.spec.vms.len() as u64 * 4096
        );
        assert_eq!(report.available_for_vms_mib, Some(6144));
        assert!(!report.warnings.is_empty());
        assert_eq!(serde_json::to_value(env).unwrap(), before);
    }
    #[test]
    fn defaults_and_unknown_host_are_explicit() {
        let mut env = environment();
        for vm in env.spec.vms.values_mut() {
            vm.memory = None;
        }
        let enough = assess_with_host_memory(&env, Some(65536));
        assert_eq!(
            enough.requested_memory_mib,
            env.spec.vms.len() as u64 * 1024
        );
        assert!(enough.warnings.is_empty());
        assert_eq!(enough.reserve_mib, 13108);
        assert!(assess_with_host_memory(&env, None)
            .available_for_vms_mib
            .is_none());
        assert_eq!(assess_with_host_memory(&env, None).warnings.len(), 1);
    }
}
