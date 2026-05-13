# **SPEC.pgRDF.LLD.v0.2**

**pgRDF: A Rust-native PostgreSQL extension for RDF/SHACL with OWL and reasoning capabilities**

*Positioning: pgRDF \- The High-Performance PostgreSQL Semantic Web Toolkit*

## **1\. Introduction**

pgRDF is a low-level PostgreSQL extension built entirely in Rust using the pgrx framework. It provides native storage and querying for RDF data directly within PostgreSQL.

**Changes in v0.2 (Performance & Ops Focus):**

* Shifts dictionary caching to PostgreSQL Shared Memory (shmem) for cross-connection cache hits.  
* Replaces raw SQL string execution with Cached Execution Plans and Custom Scan hooks.  
* Introduces declarative partitioning for the hexastore.  
* Defines a zero-install containerized deployment strategy fetching pre-compiled binaries from GitHub Releases.  
* Establishes a formal CI/CD and progression checklist.

## **2\. High-Level Architecture**

1. **Storage Engine & Shared Dictionary:** Uses a shmem-backed dictionary for microsecond ID resolution, and a partitioned hexastore architecture.  
2. **SPARQL Execution Engine:** Parses SPARQL into Abstract Syntax Trees (AST) and maps them to parameterized, prepared SQL execution plans (bypassing redundant SQL string parsing).  
3. **Inference Engine:** Streams data to the reasonable Datalog reasoner and bulk-loads (COPY) materialized inferences.  
4. **Validation Engine:** Validates RDF graphs against SHACL shape graphs (shacl-rust), outputting JSONB validation reports.

## **3\. Storage Layer Design (Native Postgres Gizmos)**

To maximize speed, pgRDF leans heavily into native Postgres optimization features.

### **3.1 Shared Dictionary Table (\_pgrdf\_dictionary)**

Stores the actual strings and maps them to unique 64-bit integers.

* **Optimization:** We use a HASH index on the string values for O(1) lookups during ingestion, combined with a standard B-Tree for ID lookups.

CREATE TABLE \_pgrdf\_dictionary (  
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,  
    term\_type SMALLINT NOT NULL,  \-- 1: URI, 2: BlankNode, 3: Literal  
    lexical\_value TEXT NOT NULL,  
    datatype\_iri\_id BIGINT,         
    language\_tag TEXT,              
    CONSTRAINT unique\_term UNIQUE (term\_type, lexical\_value, datatype\_iri\_id, language\_tag)  
);  
\-- HASH index is significantly faster for exact string matching during ingestion  
CREATE INDEX \_pgrdf\_dict\_val\_idx ON \_pgrdf\_dictionary USING HASH (lexical\_value);

### **3.2 Partitioned Quad Table (\_pgrdf\_quads)**

Stores the relationships. To handle billions of triples, the table is natively partitioned by graph\_id. This allows users to easily drop entire graphs (DROP TABLE \_pgrdf\_quads\_graph\_X) in milliseconds without triggering massive DELETE vacuums.

CREATE TABLE \_pgrdf\_quads (  
    subject\_id BIGINT NOT NULL,  
    predicate\_id BIGINT NOT NULL,  
    object\_id BIGINT NOT NULL,  
    graph\_id BIGINT NOT NULL DEFAULT 0,   
    is\_inferred BOOLEAN NOT NULL DEFAULT FALSE   
) PARTITION BY LIST (graph\_id);

\-- Default partition  
CREATE TABLE \_pgrdf\_quads\_default PARTITION OF \_pgrdf\_quads DEFAULT;

### **3.3 Covering Indices (Hexastore)**

We use standard B-Trees but utilize the INCLUDE clause. This allows Postgres to execute **Index-Only Scans**, resolving queries entirely from the index without touching the heap memory.

CREATE INDEX \_pgrdf\_idx\_spo ON \_pgrdf\_quads (subject\_id, predicate\_id, object\_id) INCLUDE (is\_inferred);  
CREATE INDEX \_pgrdf\_idx\_pos ON \_pgrdf\_quads (predicate\_id, object\_id, subject\_id) INCLUDE (is\_inferred);  
CREATE INDEX \_pgrdf\_idx\_osp ON \_pgrdf\_quads (object\_id, subject\_id, predicate\_id) INCLUDE (is\_inferred);

## **4\. Low-Level Component Design (Rust modules)**

### **4.1 Shared Dictionary Manager (src/storage/dict.rs)**

Postgres creates a new OS process for every connection. An isolated Rust LruCache would be useless.

* **v0.2 Optimization:** We use pgrx::shmem (PostgreSQL Shared Memory) to create an instance-wide RwLock\<LruCache\<u64, i64\>\> (hashing the RdfTerm to a u64 for memory efficiency).  
* **Flow:** Read from shared memory \-\> If miss, fetch from DB via Spi \-\> Insert into shared memory.

### **4.2 SPARQL Executor (src/query/executor.rs)**

Instead of generating generic SQL strings, we use **Prepared Statements**.

* **Flow:** 1\. Parse SPARQL via spargebra.  
  2\. Translate BGP (Basic Graph Patterns) into a parameterized Postgres execution plan:  
  PREPARE pgrdf\_q1 AS SELECT ... FROM \_pgrdf\_quads WHERE subject\_id \= $1...  
  3\. Cache the execution plan identifier using pgrx::Spi::prepare.  
  4\. Subsequent identical structural queries bypass the Postgres Query Planner entirely, yielding massive latency reductions.

### **4.3 Bulk Ingestion Engine (src/storage/loader.rs)**

Loading NTriples row-by-row is too slow.

* **Flow:** The loader parses files, resolves IDs via the Shmem Dictionary, buffers quads in memory, and uses Postgres' native COPY \_pgrdf\_quads FROM STDIN (FORMAT BINARY) API for millions-of-rows-per-second ingestion speeds.

## **5\. Deployment Strategy: "Zero-Install" Container Startup**

To provide a seamless, cloud-native experience (similar to AGE), pgRDF will not require users to compile Rust. It will automatically attach to official PostgreSQL containers.

### **5.1 GitHub Releases Strategy**

CI/CD builds pgrdf.tar.gz containing:

1. pgrdf.so (The compiled Rust binary)  
2. pgrdf.control (Extension metadata)  
3. pgrdf--0.2.sql (The setup SQL schema)

### **5.2 Container Entrypoint Script (init-pgrdf.sh)**

Users map a small bash script to the official Postgres docker-entrypoint-initdb.d/ directory. When the container starts, it downloads the extension dynamically.

\#\!/bin/bash  
\# /docker-entrypoint-initdb.d/00-install-pgrdf.sh

PGRDF\_VERSION=${PGRDF\_VERSION:-"latest"}  
PG\_VERSION=$(postgres \-V | grep \-oP '\\d+' | head \-1)  
ARCH=$(uname \-m)

echo "Installing pgRDF for Postgres $PG\_VERSION ($ARCH)..."

\# Fetch release URL from GitHub API  
DOWNLOAD\_URL="\[https://github.com/my-org/pgRDF/releases/latest/download/pgrdf-pg$\](https://github.com/my-org/pgRDF/releases/latest/download/pgrdf-pg$){PG\_VERSION}-${ARCH}.tar.gz"

\# Download and extract directly into Postgres library/extension directories  
curl \-sL $DOWNLOAD\_URL | tar \-xz \-C /

\# Automatically enable extension in the default database  
psql \-U "$POSTGRES\_USER" \-d "$POSTGRES\_DB" \-c "CREATE EXTENSION IF NOT EXISTS pgrdf;"

## **6\. CI/CD & Progression Strategy (GitHub Actions)**

To ensure pgRDF remains stable across Postgres versions, we utilize a strict GitHub Actions pipeline.

### **6.1 The Build Matrix**

The workflow tests and builds across combinations of:

* **PostgreSQL Versions:** 14, 15, 16, 17  
* **Architectures:** ubuntu-latest (x86\_64), ubuntu-latest via QEMU (arm64)

### **6.2 Testing Layers**

1. **Rust Unit Tests (cargo test):** Tests parsing, SHACL logic, and reasoner directly without Postgres overhead.  
2. **Integration Tests (cargo pgrx test):** Spins up a temporary Postgres cluster, installs the extension, and executes SQL tests.  
3. **W3C Compliance Tests:** A specific test suite that loads standard W3C SPARQL/SHACL test manifests into the Postgres database and verifies the output records.

## **7\. Development Checklist & Progression**

### **Phase 1: Core Storage & Build Automation (Weeks 1-2)**

* \[ \] Initialize cargo pgrx init project.  
* \[ \] Implement \_pgrdf\_dictionary and \_pgrdf\_quads schema setup in pgrdf--0.1.sql.  
* \[ \] Setup GitHub Actions matrix build to ensure successful .so compilation.  
* \[ \] Implement pgrdf\_load\_file() using native Postgres COPY BINARY via pgrx for high-speed ingestion.

### **Phase 2: Query Engine & Shared Memory (Weeks 3-4)**

* \[ \] Implement the pgrx::shmem Dictionary Cache (Lock-free or RwLock).  
* \[ \] Integrate spargebra and map BGP AST to parameterized SQL (Spi::prepare).  
* \[ \] Implement sparql(query TEXT) UDF returning set of rows.  
* \[ \] Add W3C SPARQL evaluation tests to CI/CD.

### **Phase 3: Semantic Engine (Weeks 5-6)**

* \[ \] Wrap the reasonable crate inside pgrdf\_materialize().  
* \[ \] Stream database rows into the reasoner and COPY back inferred quads.  
* \[ \] Wrap shacl-rust inside pgrdf\_validate() returning JSONB.  
* \[ \] Add SHACL validation tests to CI/CD.

### **Phase 4: Release & Containerization (Week 7\)**

* \[ \] Finalize pgrx package structure for releases.  
* \[ \] Create the GitHub Action step to push tarballs (.so, .control, .sql) to GitHub Releases on tag creation.  
* \[ \] Publish the standard init-pgrdf.sh script for community container usage.  
* \[ \] Benchmark against Apache Jena TDB and AGE using the LUBM benchmark.