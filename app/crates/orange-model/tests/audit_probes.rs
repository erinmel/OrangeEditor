mod common;

use common::{content, empty_document, id_source, nid, node};
use common::generic::{catalog, core_converters, document, ready_plan, typed_sink, typed_source};
use orange_model::{BodyRef, DocCommand, GraphDocument, LayoutCommand, from_json};
use orange_model::conversion::{PlanApplyError, apply_connection};
use orange_model::paste::Fragment;
use serde_json::json;

#[test]
fn revision_exhaustion_does_not_panic_or_mutate() {
    let value = serde_json::to_value(empty_document()).unwrap();
    let mut value = value.as_object().unwrap().clone();
    value.insert("format".into(), json!("orange.graph"));
    value.insert("revision".into(), json!(u64::MAX));
    let mut doc = from_json(&serde_json::to_string(&value).unwrap()).unwrap();
    let before = content(&doc);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        doc.apply([DocCommand::Layout(LayoutCommand::Set { path: vec!["probe".into()], value: Some(json!(true)) })])
    }));
    eprintln!("panic={}, content_changed={}, revision={}", result.is_err(), before != content(&doc), doc.revision());
    assert!(result.is_ok(), "untrusted revision panics during commit");
    assert!(result.unwrap().is_err());
    assert_eq!(content(&doc), before);
}

#[test]
fn public_document_mutation_invalidates_connection_plan() {
    let registry = core_converters();
    let mut doc = document(vec![typed_source(1,"i64"), typed_sink(3,"i64",json!({}))],vec![]);
    let plan = ready_plan(&doc,&registry,(1,"value"),(3,"value"));
    doc.extensions.insert("org.test/semantic".into(), json!(true));
    let result = apply_connection(&mut doc, &registry, &catalog(None), plan);
    assert!(matches!(result, Err(PlanApplyError::StaleDocument { .. })), "{result:?}");
}

#[test]
fn moving_ids_between_bodies_keeps_incremental_index_equivalent() {
    use common::{body,function,function_id};
    let mut doc = empty_document();
    let _ = doc.apply([DocCommand::AddNode { body:BodyRef::Root,node:node(1) }]).unwrap();
    let mut index = orange_model::DocIndex::build(&doc);
    let done = doc.apply([
        DocCommand::RemoveNodes { ids:vec![nid(1)] },
        DocCommand::UpsertFunction(function(body([node(1)],[])))
    ]).unwrap();
    index.update(&doc,&done.change_set);
    assert_eq!(doc.locate_node(nid(1)),Some(BodyRef::Function(function_id())));
    assert_eq!(index,orange_model::DocIndex::build(&doc));
}

#[test]
fn fragment_paste_rekeys_signatures_from_loaded_document() {
    let doc = document(vec![typed_source(1,"i64")], vec![]);
    let fragment = Fragment::copy(&doc,&[nid(1)]).unwrap();
    let mut target: GraphDocument = empty_document();
    let paste=fragment.paste(&target,BodyRef::Root,id_source(5)).unwrap();
    let _=target.apply(paste.commands).unwrap();
}

#[test]
fn shared_nominal_schema_proof_cost() {
    use orange_model::types::{Constraint, TypeRegistryBuilder};
    use orange_model::type_term::TypeTerm;
    use orange_model::value::literal_codec::{FieldSchema,RecordSchema};
    for depth in [8,12,16] {
        let mut builder=TypeRegistryBuilder::core();
        for n in 0..depth {
            let child=if n==0 {"core/i64@1".to_owned()} else {format!("org.audit/t{}@1",n-1)};
            let child=TypeTerm::parse(&child.parse().unwrap()).unwrap();
            let ty=format!("org.audit/t{n}@1").parse().unwrap();
            builder.register_record("org.audit", &ty,RecordSchema {fields:vec![
                FieldSchema {field_id:"left".into(),ty:child.clone(),optional:false},
                FieldSchema {field_id:"right".into(),ty:child,optional:false},
            ]}).unwrap();
            builder.derive_constraint("org.audit",&ty,Constraint::Eq).unwrap();
        }
        let start=std::time::Instant::now();
        let _registry=builder.build().unwrap();
        eprintln!("nominal_depth={depth}, build_us={}",start.elapsed().as_micros());
    }
}
