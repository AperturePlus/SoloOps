use std::{fs, path::PathBuf};

use soloops_domain::RUN_STATUSES;

fn workspace_file(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(path)).unwrap()
}

#[test]
fn frontend_and_openapi_include_every_run_status() {
    let frontend = workspace_file("apps/web/src/lib/contracts.ts");
    let openapi = workspace_file("docs/api/openapi.yaml");
    for status in RUN_STATUSES {
        let value = status.as_str();
        assert!(
            frontend.contains(&format!("\"{value}\"")),
            "frontend contract is missing {value}"
        );
        assert!(
            openapi.contains(&format!("- {value}")),
            "OpenAPI contract is missing {value}"
        );
    }
}
