-- Seed the `pgrdf_demo` (public tier) project with a small LUBM-flavoured
-- university dataset so the Console has real classes / predicates / triples
-- to render. Idempotent-ish: clears graph 1 first.
--
--   docker exec -i pgrdf-local-pg psql -U pgrdf -d pgrdf_demo < sql/010-seed-demo.sql

SELECT pgrdf.add_graph(1, 'urn:demo:lubm') WHERE pgrdf.graph_iri(1) IS NULL;
SELECT pgrdf.clear_graph(1);

SELECT pgrdf.parse_turtle($ttl$
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ub:   <http://swat.cse.lehigh.edu/onto/univ-bench.owl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix inst: <http://example.org/lubm/> .

# ---- class lattice ----
ub:Person        a rdfs:Class .
ub:Faculty       rdfs:subClassOf ub:Person .
ub:Professor     rdfs:subClassOf ub:Faculty .
ub:FullProfessor rdfs:subClassOf ub:Professor .
ub:Student       rdfs:subClassOf ub:Person .
ub:GraduateStudent rdfs:subClassOf ub:Student .
ub:Course        a rdfs:Class .
ub:GraduateCourse rdfs:subClassOf ub:Course .
ub:Department    a rdfs:Class .

# ---- departments ----
inst:CS  a ub:Department ; foaf:name "Computer Science" .
inst:EE  a ub:Department ; foaf:name "Electrical Engineering" .

# ---- professors ----
inst:Prof0 a ub:FullProfessor ; foaf:name "Allen Pereira"    ; ub:worksFor inst:CS ;
           ub:teacherOf inst:CS401 , inst:CS512 .
inst:Prof2 a ub:Professor     ; foaf:name "Beatrice Marchand" ; ub:worksFor inst:CS ;
           ub:teacherOf inst:CS512 .
inst:Prof4 a ub:FullProfessor ; foaf:name "Chen Wei-Lin"      ; ub:worksFor inst:EE ;
           ub:teacherOf inst:AI610 .
inst:Prof7 a ub:Professor     ; foaf:name "Dimitri Kovac"     ; ub:worksFor inst:CS ;
           ub:teacherOf inst:DB320 .

# ---- courses ----
inst:CS401 a ub:Course         ; foaf:name "Intro to Databases" .
inst:CS512 a ub:GraduateCourse ; foaf:name "Advanced Query Engines" .
inst:AI610 a ub:GraduateCourse ; foaf:name "Knowledge Representation" .
inst:DB320 a ub:Course         ; foaf:name "Storage Internals" .

# ---- graduate students ----
inst:Stud0421 a ub:GraduateStudent ; foaf:name "Esha Raghavan" ;
              ub:takesCourse inst:CS401 , inst:CS512 ; ub:advisor inst:Prof0 .
inst:Stud1041 a ub:GraduateStudent ; foaf:name "Felix Hartmann" ;
              ub:takesCourse inst:CS401 ; ub:advisor inst:Prof0 .
inst:Stud0188 a ub:GraduateStudent ; foaf:name "Gita Saraswati" ;
              ub:takesCourse inst:CS512 ; ub:advisor inst:Prof2 .
inst:Stud2104 a ub:GraduateStudent ; foaf:name "Hiroshi Tanaka" ;
              ub:takesCourse inst:AI610 ; ub:advisor inst:Prof4 .
inst:Stud2666 a ub:GraduateStudent ; foaf:name "Ioana Voicu" ;
              ub:takesCourse inst:DB320 , inst:AI610 ; ub:advisor inst:Prof7 .
$ttl$, 1);

SELECT 'seeded demo graph 1, quads=' || pgrdf.count_quads(1) AS result;
