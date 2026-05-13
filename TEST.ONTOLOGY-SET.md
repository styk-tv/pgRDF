# Ontology fetch set for pgRDF Phase 2.1+.
#
# These ontologies are work-in-progress in some cases and may contain
# authoring errors. A parse failure from pgrdf.load_turtle on any of
# them is signal — oxttl 0.2.x is strict about RFC 3987 IRIs and the
# Turtle 1.1 grammar, so anything it rejects is genuinely off-spec.
# Run `tests/perf/smoke-ontologies.sh` to see the current pass/fail
# map and per-ontology triple counts.
#
# Known issues from 2026-05-13 fetch:
#   - prov.ttl: relative IRIs (`<#>`); pass base_iri to load_turtle.
#   - workflow.ttl: `<ckp://Name:v0.1>` IRI form has a colon in the
#     path segment (after the host), which RFC 3986 §3.3 doesn't
#     permit. The CKP source needs to either escape the colon
#     (`%3A`) or switch to a fragment/dot-separated form.
#
# ConceptKernel v3.7 ontology family (12 modules).
https://conceptkernel.org/ontology/v3.7/core.ttl
https://conceptkernel.org/ontology/v3.7/base-instances.ttl
https://conceptkernel.org/ontology/v3.7/proof.ttl
https://conceptkernel.org/ontology/v3.7/edges.ttl
https://conceptkernel.org/ontology/v3.7/consensus.ttl
https://conceptkernel.org/ontology/v3.7/kernel-metadata.ttl
https://conceptkernel.org/ontology/v3.7/processes.ttl
https://conceptkernel.org/ontology/v3.7/relations.ttl
https://conceptkernel.org/ontology/v3.7/rbac.ttl
https://conceptkernel.org/ontology/v3.7/workflow.ttl
https://conceptkernel.org/ontology/v3.7/self-improvement.ttl
https://conceptkernel.org/ontology/v3.7/shapes.ttl

# (BFO-2020 bfo-core.ttl and CCO AllCoreOntology.ttl previously here
# returned 404 from their stated GitHub paths. Replace with current
# paths or drop when verified.)
# https://raw.githubusercontent.com/BFO-ontology/BFO-2020/master/src/owl/bfo-core.ttl
# https://raw.githubusercontent.com/CommonCoreOntology/CommonCoreOntologies/master/src/cco-modules/AllCoreOntology.ttl

# W3C standard vocabularies.
https://www.w3.org/ns/prov.ttl
https://www.w3.org/ns/prov-o.ttl
https://www.w3.org/2000/01/rdf-schema
https://www.w3.org/2002/07/owl
https://www.w3.org/2006/vcard/ns
https://www.w3.org/ns/odrl/2/ODRL22.ttl
https://www.w3.org/ns/shacl.ttl
https://www.w3.org/ns/dcat

# W3C SDW (Sensor + Time) — published in their gh-pages branch as TTL.
https://raw.githubusercontent.com/w3c/sdw/gh-pages/time/rdf/time.ttl
https://raw.githubusercontent.com/w3c/sdw/gh-pages/ssn/integrated/sosa.ttl
https://raw.githubusercontent.com/w3c/sdw/gh-pages/ssn/ssn_separated/ssn.ttl

# FOAF — Apache Jena keeps a curated Turtle copy.
https://raw.githubusercontent.com/apache/jena/main/jena-arq/Vocabularies/FOAF.ttl

# ValueFlows.
https://codeberg.org/valueflows/pages/raw/branch/main/assets/all_vf.TTL
