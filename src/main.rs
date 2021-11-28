use std::str;

#[macro_use] extern crate log;
use anyhow::Result;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
    CustomResourceDefinition, CustomResourceDefinitionVersion,
};
use kopium::{analyze, OutputStruct};
use kube::{Api, Client, ResourceExt};
use quote::format_ident;
use structopt::StructOpt;

const KEYWORDS: [&str; 23] = [
    "for", "impl", "continue", "enum", "const", "break", "as", "move", "mut", "mod", "pub", "ref", "self",
    "static", "struct", "super", "true", "trait", "type", "unsafe", "use", "where", "while",
];


#[derive(StructOpt, Debug)]
#[structopt(
    version = clap::crate_version!(),
    author = "clux <sszynrae@gmail.com>",
    about = "Kubernetes OPenapI UnMangler",
)]
struct Kopium {
    #[structopt(about = "Give the name of the input CRD to use e.g. prometheusrules.monitoring.coreos.com")]
    crd: String,
    #[structopt(about = "Use this CRD version if multiple versions are present", long)]
    api_version: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let kopium = Kopium::from_args();
    let client = Client::try_default().await?;
    let api: Api<CustomResourceDefinition> = Api::all(client);
    let crd = api.get(&kopium.crd).await?;

    let version = find_crd_version(&crd, kopium.api_version.as_deref())?;
    let data = version
        .schema
        .as_ref()
        .and_then(|schema| schema.open_api_v3_schema.clone());
    let version = version.name.clone();

    let kind = crd.spec.names.kind;
    let plural = crd.spec.names.plural;
    let group = crd.spec.group;
    let scope = crd.spec.scope;


    if let Some(schema) = data {
        let mut results = vec![];
        debug!("schema: {}", serde_json::to_string_pretty(&schema)?);
        analyze(schema, "", &kind, 0, &mut results)?;

        print_prelude(&results);
        for s in results {
            if s.level == 0 {
                continue; // ignoring root struct
            } else {
                if s.level == 1 && s.name.ends_with("Spec") {
                    println!("#[derive(CustomResource, Serialize, Deserialize, Clone, Debug)]");
                    println!(
                        r#"#[kube(group = "{}", version = "{}", kind = "{}", plural = "{}")]"#,
                        group, version, kind, plural
                    );
                    if scope == "Namespaced" {
                        println!(r#"#[kube(namespaced)]"#);
                    }
                    // don't support grabbing original schema atm so disable schemas:
                    // (we coerce IntToString to String anyway so it wont match anyway)
                    println!(r#"#[kube(schema = "disabled")]"#);
                    println!("pub struct {} {{", s.name);
                } else {
                    println!("#[derive(Serialize, Deserialize, Clone, Debug)]");
                    let spec_trimmed_name = s.name.as_str().replace(&format!("{}Spec", kind), &kind);
                    println!("pub struct {} {{", spec_trimmed_name);
                }
                for m in s.members {
                    if let Some(annot) = m.field_annot {
                        println!("    {}", annot);
                    }
                    let safe_name = if KEYWORDS.contains(&m.name.as_ref()) {
                        format_ident!("r#{}", m.name)
                    } else {
                        format_ident!("{}", m.name)
                    };
                    let spec_trimmed_type = m.type_.as_str().replace(&format!("{}Spec", kind), &kind);
                    println!("    pub {}: {},", safe_name, spec_trimmed_type);
                }
                println!("}}")
            }
        }
    } else {
        error!("no schema found for crd {}", kopium.crd);
    }

    Ok(())
}

fn print_prelude(results: &[OutputStruct]) {
    println!("use kube::CustomResource;");
    println!("use serde::{{Serialize, Deserialize}};");
    if results.iter().any(|o| o.uses_btreemaps()) {
        println!("use std::collections::BTreeMap;");
    }
    if results.iter().any(|o| o.uses_datetime()) {
        println!("use chrono::{{DateTime, Utc}};");
    }
    if results.iter().any(|o| o.uses_date()) {
        println!("use chrono::naive::NaiveDate;");
    }
    println!();
}

fn find_crd_version<'a>(
    crd: &'a CustomResourceDefinition,
    version: Option<&str>,
) -> Result<&'a CustomResourceDefinitionVersion> {
    if let Some(version) = version {
        crd.spec
            .versions
            .iter()
            .find(|v| v.name == version)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Version '{}' not found in CRD '{}'\navailable versions are '{}'",
                    version,
                    crd.name(),
                    all_versions(crd)
                )
            })
    } else {
        crd.spec
            .versions
            .first()
            .ok_or_else(|| anyhow::anyhow!("CRD '{}' has no versions", crd.name()))
    }
}

fn all_versions(crd: &CustomResourceDefinition) -> String {
    crd.spec
        .versions
        .iter()
        .map(|v| v.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
