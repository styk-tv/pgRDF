// Sample LUBM-flavoured RDF data for pgRDF Console demo.
// All data lives in-memory; mirrors what a small research_db instance might hold.

const PREFIXES = {
  rdf:  "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
  rdfs: "http://www.w3.org/2000/01/rdf-schema#",
  owl:  "http://www.w3.org/2002/07/owl#",
  xsd:  "http://www.w3.org/2001/XMLSchema#",
  sh:   "http://www.w3.org/ns/shacl#",
  ub:   "http://swat.cse.lehigh.edu/onto/univ-bench.owl#",
  inst: "http://example.org/lubm/",
  foaf: "http://xmlns.com/foaf/0.1/",
};

const GRAPHS = [
  { id: 0, name: "default",           label: "default",            triples: 1_204_881 },
  { id: 1, name: "lubm:2024",         label: "lubm:2024",          triples:   848_120 },
  { id: 2, name: "lubm:inferred",     label: "lubm:inferred",      triples:   312_445, inferred: true },
  { id: 3, name: "dbpedia:sample",    label: "dbpedia:sample",     triples:    44_316 },
];

const CLASSES = [
  { iri: "ub:University",        instances: 12,     subOf: null,             color: "graph" },
  { iri: "ub:Department",        instances: 96,     subOf: "ub:Organization", color: "graph" },
  { iri: "ub:Organization",      instances: 108,    subOf: null,             color: "graph" },
  { iri: "ub:Person",            instances: 8_412,  subOf: null,             color: "class" },
  { iri: "ub:Faculty",           instances: 1_204,  subOf: "ub:Person",      color: "class" },
  { iri: "ub:Professor",         instances: 612,    subOf: "ub:Faculty",     color: "class" },
  { iri: "ub:AssociateProf",     instances: 287,    subOf: "ub:Professor",   color: "class" },
  { iri: "ub:FullProfessor",     instances: 188,    subOf: "ub:Professor",   color: "class" },
  { iri: "ub:Student",           instances: 6_996,  subOf: "ub:Person",      color: "class" },
  { iri: "ub:UndergradStudent",  instances: 4_201,  subOf: "ub:Student",     color: "class" },
  { iri: "ub:GraduateStudent",   instances: 2_795,  subOf: "ub:Student",     color: "class" },
  { iri: "ub:Course",            instances: 1_802,  subOf: null,             color: "class" },
  { iri: "ub:GraduateCourse",    instances: 612,    subOf: "ub:Course",      color: "class" },
  { iri: "ub:Publication",       instances: 9_124,  subOf: null,             color: "class" },
  { iri: "ub:ResearchGroup",     instances: 48,     subOf: "ub:Organization", color: "graph" },
];

const PREDICATES = [
  { iri: "rdf:type",          uses: 18_241, domain: null,             range: "rdfs:Class",   group: "rdf"  },
  { iri: "rdfs:label",        uses: 12_840, domain: null,             range: "xsd:string",   group: "rdfs" },
  { iri: "ub:worksFor",       uses:  1_204, domain: "ub:Faculty",     range: "ub:Organization", group: "ub" },
  { iri: "ub:teacherOf",      uses:  2_811, domain: "ub:Faculty",     range: "ub:Course",    group: "ub"   },
  { iri: "ub:takesCourse",    uses:  9_412, domain: "ub:Student",     range: "ub:Course",    group: "ub"   },
  { iri: "ub:advisor",        uses:  2_795, domain: "ub:GraduateStudent", range: "ub:Professor", group: "ub" },
  { iri: "ub:memberOf",       uses:  7_140, domain: "ub:Person",      range: "ub:Organization", group: "ub" },
  { iri: "ub:headOf",         uses:     96, domain: "ub:Professor",   range: "ub:Department", group: "ub"  },
  { iri: "ub:subOrganizationOf", uses: 144, domain: "ub:Organization", range: "ub:Organization", group: "ub" },
  { iri: "ub:publicationAuthor", uses: 11_842, domain: "ub:Publication", range: "ub:Person", group: "ub" },
  { iri: "ub:emailAddress",   uses:  8_412, domain: "ub:Person",      range: "xsd:string",   group: "ub"   },
  { iri: "ub:age",            uses:  6_996, domain: "ub:Student",     range: "xsd:integer",  group: "ub"   },
  { iri: "foaf:name",         uses:  8_120, domain: "ub:Person",      range: "xsd:string",   group: "foaf" },
];

const SHAPES = [
  { iri: "ex:ProfessorShape",  targets: "ub:Professor",      constraints: 5, severity: "Violation" },
  { iri: "ex:CourseShape",     targets: "ub:Course",         constraints: 3, severity: "Violation" },
  { iri: "ex:GradStudentShape",targets: "ub:GraduateStudent",constraints: 4, severity: "Warning"   },
  { iri: "ex:EmailShape",      targets: "ub:Person",         constraints: 1, severity: "Violation" },
  { iri: "ex:PublicationShape",targets: "ub:Publication",    constraints: 2, severity: "Warning"   },
];

const RULES = [
  { iri: "rdfs:subClassOf",   kind: "RDFS",   derives: 184_211, status: "materialized" },
  { iri: "rdfs:domain",       kind: "RDFS",   derives:  98_402, status: "materialized" },
  { iri: "rdfs:range",        kind: "RDFS",   derives:  72_118, status: "materialized" },
  { iri: "owl:TransitiveProperty", kind: "OWL2-RL", derives: 28_491, status: "materialized" },
  { iri: "owl:inverseOf",     kind: "OWL2-RL", derives:  4_204, status: "stale" },
  { iri: "owl:sameAs",        kind: "OWL2-RL", derives:  1_018, status: "disabled" },
];

const SAVED = [
  { id: "q1", name: "Profs with grad advisees",   updated: "2m",  vars: 3 },
  { id: "q2", name: "Course load — Fall 2024",    updated: "1h",  vars: 4 },
  { id: "q3", name: "Co-authorship clusters",     updated: "3h",  vars: 5 },
  { id: "q4", name: "Orphan publications",        updated: "1d",  vars: 2 },
  { id: "q5", name: "SHACL: bad email patterns",  updated: "2d",  vars: 3 },
];

const HISTORY = [
  { id: "h1", t: "12:42:08", q: "SELECT ?p ?c WHERE { ?p a ub:Professor ; ub:teacherOf ?c }", ms: 18, rows: 487 },
  { id: "h2", t: "12:39:51", q: "SELECT (COUNT(*) AS ?n) WHERE { ?s a ub:GraduateStudent }",  ms:  4, rows:   1 },
  { id: "h3", t: "12:14:02", q: "DESCRIBE ub:Department",                                      ms:  9, rows:  42 },
];

// Demo query nodes / edges — built to mirror the canvas display.
const Q_NODES = [
  { id: "prof",    kind: "var",  label: "?prof",    type: "ub:Professor",          x: 220, y: 110, projected: 1 },
  { id: "course",  kind: "var",  label: "?course",  type: "ub:Course",             x: 500, y: 110, projected: 2 },
  { id: "student", kind: "var",  label: "?student", type: "ub:GraduateStudent",    x: 500, y: 250, projected: 3 },
  { id: "name",    kind: "var",  label: "?name",    type: "xsd:string",            x: 220, y: 250, projected: 4 },
];

const Q_EDGES = [
  { id: "e1", from: "prof",    to: "course",  pred: "ub:teacherOf"  },
  { id: "e2", from: "student", to: "course",  pred: "ub:takesCourse" },
  { id: "e3", from: "student", to: "prof",    pred: "ub:advisor",  optional: true },
  { id: "e4", from: "prof",    to: "name",    pred: "foaf:name"     },
];

// Demo result rows for the demo query.
const RESULT_COLS = [
  { v: "prof",    label: "?prof",    type: "iri" },
  { v: "name",    label: "?name",    type: "lit" },
  { v: "course",  label: "?course",  type: "iri" },
  { v: "student", label: "?student", type: "iri" },
];

const RESULT_ROWS = [
  ["inst:Prof0",  "Allen Pereira",      "inst:Course-CS401",  "inst:Stud0421"],
  ["inst:Prof0",  "Allen Pereira",      "inst:Course-CS401",  "inst:Stud1041"],
  ["inst:Prof2",  "Béatrice Marchand",  "inst:Course-CS512",  "inst:Stud0188"],
  ["inst:Prof2",  "Béatrice Marchand",  "inst:Course-CS512",  "inst:Stud0244"],
  ["inst:Prof4",  "Chen Wei-Lin",       "inst:Course-AI610",  "inst:Stud2104"],
  ["inst:Prof4",  "Chen Wei-Lin",       "inst:Course-AI610",  "inst:Stud2218"],
  ["inst:Prof4",  "Chen Wei-Lin",       "inst:Course-AI610",  "inst:Stud2402"],
  ["inst:Prof7",  "Dimitri Kovac",      "inst:Course-DB320",  "inst:Stud1801"],
  ["inst:Prof9",  "Esha Raghavan",      "inst:Course-ML720",  "inst:Stud2666"],
  ["inst:Prof9",  "Esha Raghavan",      "inst:Course-ML720",  "inst:Stud2701"],
  ["inst:Prof11", "Felix Hartmann",     "inst:Course-NLP540", "inst:Stud2155"],
  ["inst:Prof14", "Gita Saraswati",     "inst:Course-HCI230", "inst:Stud0922"],
  ["inst:Prof14", "Gita Saraswati",     "inst:Course-HCI230", "inst:Stud1188"],
  ["inst:Prof16", "Hiroshi Tanaka",     "inst:Course-CS401",  "inst:Stud0421"],
  ["inst:Prof21", "Ioana Voicu",        "inst:Course-AI610",  "inst:Stud2402"],
  ["inst:Prof21", "Ioana Voicu",        "inst:Course-SE410",  "inst:Stud1701"],
];

const PLAN_ROWS = [
  { op: "IndexOnlyScan",  detail: "_pgrdf_idx_pos (predicate_id, object_id, subject_id)", rows: 612,  ms: 0.84, indent: 0 },
  { op: "HashJoin",       detail: "?prof.subject_id = teacherOf.subject_id",              rows: 2811, ms: 1.21, indent: 1 },
  { op: "IndexOnlyScan",  detail: "_pgrdf_idx_pos pred=ub:teacherOf",                      rows: 2811, ms: 0.92, indent: 2 },
  { op: "HashJoin",       detail: "teacherOf.object_id = takesCourse.object_id",          rows: 9412, ms: 2.18, indent: 1 },
  { op: "IndexOnlyScan",  detail: "_pgrdf_idx_pos pred=ub:takesCourse",                    rows: 9412, ms: 1.83, indent: 2 },
  { op: "HashSemiJoin",   detail: "?student ∈ ub:GraduateStudent",                        rows: 2795, ms: 0.91, indent: 1 },
  { op: "ShmemDictLookup",detail: "resolve 4 result IRIs via shared dictionary",          rows:   16, ms: 0.04, indent: 0 },
];

const LOG_ROWS = [
  { t: "12:42:08.211", lv: "info", msg: "Parsing SPARQL via spargebra", src: "executor.rs:42" },
  { t: "12:42:08.212", lv: "info", msg: "Mapped BGP to prepared plan pgrdf_q_2f9c (3 joins)", src: "executor.rs:118" },
  { t: "12:42:08.212", lv: "ok",   msg: "Cache HIT — plan reused (saved planner: ~6.2ms)", src: "executor.rs:124" },
  { t: "12:42:08.214", lv: "info", msg: "Shmem dict: 4 hits / 0 misses", src: "dict.rs:88" },
  { t: "12:42:08.229", lv: "ok",   msg: "Query OK — 16 rows in 18.04 ms", src: "executor.rs:201" },
  { t: "12:42:09.140", lv: "warn", msg: "Stale inference: owl:inverseOf last materialized 4h ago", src: "infer.rs:64" },
];

window.PGRDF_DATA = {
  PREFIXES, GRAPHS, CLASSES, PREDICATES, SHAPES, RULES, SAVED, HISTORY,
  Q_NODES, Q_EDGES, RESULT_COLS, RESULT_ROWS, PLAN_ROWS, LOG_ROWS,
};
