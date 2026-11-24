//! Whole-graph reads: the fixtures every whole-graph function is pinned
//! against.

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
    const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

    /// One term of a fixture triple.
    #[derive(Clone, Copy)]
    pub(crate) enum T {
        /// An IRI.
        I(&'static str),
        /// A blank node with this stored label.
        B(&'static str),
        /// A literal stored the way the loaders store a simple literal
        /// (datatype xsd:string).
        L(&'static str),
        /// A literal stored with no datatype and no language.
        Bare(&'static str),
        /// A typed literal.
        D(&'static str, &'static str),
        /// A language-tagged literal.
        Lang(&'static str, &'static str),
    }
    use T::*;

    /// The dictionary id of a term, inserting the row when it is not
    /// there. Plain SQL on the dictionary, so a fixture stores exactly the
    /// rows it names whatever the ingest code does.
    fn dict_id(ty: i16, val: &str, dt: Option<i64>, lang: Option<&str>) -> i64 {
        let found: Option<i64> = Spi::get_one_with_args(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary \
                      WHERE term_type = $1 AND lexical_value = $2 \
                        AND datatype_iri_id IS NOT DISTINCT FROM $3 \
                        AND language_tag IS NOT DISTINCT FROM $4 \
                      ORDER BY id LIMIT 1)",
            &[ty.into(), val.into(), dt.into(), lang.into()],
        )
        .unwrap();
        if let Some(id) = found {
            return id;
        }
        Spi::get_one_with_args(
            "INSERT INTO pgrdf._pgrdf_dictionary \
               (term_type, lexical_value, datatype_iri_id, language_tag) \
             VALUES ($1, $2, $3, $4) RETURNING id",
            &[ty.into(), val.into(), dt.into(), lang.into()],
        )
        .unwrap()
        .unwrap()
    }

    pub(crate) fn term_id(t: T) -> i64 {
        match t {
            I(v) => dict_id(1, v, None, None),
            B(v) => dict_id(2, v, None, None),
            L(v) => {
                let dt = dict_id(1, XSD_STRING, None, None);
                dict_id(3, v, Some(dt), None)
            }
            Bare(v) => dict_id(3, v, None, None),
            D(v, d) => {
                let dt = dict_id(1, d, None, None);
                dict_id(3, v, Some(dt), None)
            }
            Lang(v, l) => dict_id(3, v, None, Some(l)),
        }
    }

    /// Store one row of a fixture.
    pub(crate) fn put(graph_id: i64, s: T, p: T, o: T, inferred: bool) {
        let (s, p, o) = (term_id(s), term_id(p), term_id(o));
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_quads \
               (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                s.into(),
                p.into(),
                o.into(),
                graph_id.into(),
                inferred.into(),
            ],
        )
        .unwrap();
    }

    pub(crate) fn fill(graph_id: i64, triples: &[(T, T, T)]) {
        for (s, p, o) in triples {
            put(graph_id, *s, *p, *o, false);
        }
    }

    pub(crate) fn new_graph(graph_id: i64) {
        Spi::run(&format!("SELECT pgrdf.add_graph({graph_id})")).unwrap();
    }

    // ── fixtures ───────────────────────────────────────────────────────

    /// Ground triples only: every literal form, the escapes, non-ASCII,
    /// and the pairs where one serialised term is a prefix of another.
    pub(crate) fn ground_triples() -> Vec<(T, T, T)> {
        let p = I("urn:wg:p");
        vec![
            (I("urn:wg:s1"), p, I("urn:wg:o1")),
            (I("urn:wg:s1"), p, I("urn:wg:o10")),
            (I("urn:wg:s10"), p, I("urn:wg:o1")),
            (I("urn:wg:s1"), p, L("x")),
            (I("urn:wg:s1"), p, L("x y")),
            (I("urn:wg:s1"), p, Lang("x", "en")),
            (I("urn:wg:s1"), p, Lang("x", "en-GB")),
            (I("urn:wg:s1"), p, D("x", "urn:wg:dt")),
            (I("urn:wg:s1"), p, Bare("bare")),
            (I("urn:wg:s1"), p, L("")),
            (I("urn:wg:s1"), p, D("42", XSD_INTEGER)),
            (I("urn:wg:s1"), I("urn:wg:p2"), L("x")),
            (
                I("urn:wg:s2"),
                p,
                L("line1\nline2\ttab\rcr \"quoted\" back\\slash"),
            ),
            (I("urn:wg:s2"), p, L("a\" .")),
            (
                I("urn:wg:s2"),
                p,
                L("h\u{e9}llo w\u{f6}rld \u{2014} \u{65e5}\u{672c}\u{8a9e} \u{1f389}"),
            ),
            (I("urn:wg:s2"), p, L("ctl\u{1}and\u{7f}del")),
            (I("urn:wg:\u{fc}"), p, I("urn:wg:\u{65e5}\u{672c}")),
            (I("urn:wg:s2"), I("urn:wg:q"), Lang("\u{e9}t\u{e9}", "fr")),
        ]
    }

    /// Blank nodes with fixed stored labels, in subject and object
    /// position: shared, symmetric, a 4-cycle (needs the n-degree hash),
    /// a label that is a prefix of another, and ground triples that sort
    /// between the blank-node lines.
    pub(crate) fn bnode_triples() -> Vec<(T, T, T)> {
        let p = I("urn:wg:p");
        let q = I("urn:wg:q");
        vec![
            (I("urn:wg:s1"), p, I("urn:wg:o")),
            (I("urn:wg:s1"), p, B("b1")),
            (I("urn:wg:s1"), p, L("lit")),
            (I("urn:wg:s1"), q, L("z")),
            (I("urn:wg:s2"), q, B("sh")),
            (I("urn:wg:s3"), q, B("sh")),
            (B("sh"), p, Lang("v", "en")),
            (B("b1"), p, I("urn:wg:o")),
            (B("b1"), p, L("t\tab")),
            (B("b10"), p, I("urn:wg:o")),
            (B("b2"), p, B("b3")),
            (B("b3"), p, B("b2")),
            (B("c1"), q, B("c2")),
            (B("c2"), q, B("c3")),
            (B("c3"), q, B("c4")),
            (B("c4"), q, B("c1")),
            (B("c1"), p, L("marked")),
            (B("lone"), p, B("lone")),
        ]
    }

    /// A small graph for the default graph, the inferred-rows fixture and
    /// the locked one.
    pub(crate) fn small_triples() -> Vec<(T, T, T)> {
        let p = I("urn:wg:p");
        vec![
            (I("urn:wg:a"), p, I("urn:wg:b")),
            (I("urn:wg:a"), p, L("one")),
            (I("urn:wg:a"), p, B("k")),
            (B("k"), p, D("7", XSD_INTEGER)),
        ]
    }

    /// What the whole-graph functions say about one graph.
    #[derive(Debug, PartialEq)]
    pub(crate) struct Reading {
        pub graph_digest: String,
        pub structural_digest: String,
        pub manifest_bytes: String,
        pub manifest_identity: String,
        pub manifest_structure: String,
        pub asserted: i64,
        pub inferred: i64,
        pub export: Vec<String>,
    }

    pub(crate) fn export(graph_id: i64) -> Vec<String> {
        let mut out = Vec::new();
        Spi::connect(|c| {
            for r in c
                .select(
                    "SELECT l FROM pgrdf.export_graph($1) WITH ORDINALITY AS e(l, n) ORDER BY n",
                    None,
                    &[graph_id.into()],
                )
                .unwrap()
            {
                out.push(r.get::<String>(1).unwrap().unwrap());
            }
        });
        out
    }

    pub(crate) fn manifest(graph_id: i64) -> serde_json::Value {
        Spi::get_one_with_args::<pgrx::JsonB>("SELECT pgrdf.graph_manifest($1)", &[graph_id.into()])
            .unwrap()
            .unwrap()
            .0
    }

    pub(crate) fn read(graph_id: i64) -> Reading {
        let one = |f: &str| -> String {
            Spi::get_one_with_args(&format!("SELECT pgrdf.{f}($1)"), &[graph_id.into()])
                .unwrap()
                .unwrap()
        };
        let m = manifest(graph_id);
        let s = |v: &serde_json::Value| v.as_str().unwrap().to_string();
        Reading {
            graph_digest: one("graph_digest"),
            structural_digest: one("structural_digest"),
            manifest_bytes: s(&m["digests"]["bytes"]["value"]),
            manifest_identity: s(&m["digests"]["identity"]["value"]),
            manifest_structure: s(&m["digests"]["structure"]["value"]),
            asserted: m["counts"]["asserted"].as_i64().unwrap(),
            inferred: m["counts"]["inferred"].as_i64().unwrap(),
            export: export(graph_id),
        }
    }

    /// Build every pinned fixture and return its reading, by name.
    fn build_all() -> Vec<(&'static str, Reading)> {
        let mut out = Vec::new();

        new_graph(986101);
        fill(986101, &ground_triples());
        out.push(("ground", read(986101)));

        new_graph(986102);
        fill(986102, &bnode_triples());
        out.push(("bnodes", read(986102)));

        new_graph(986103);
        out.push(("empty", read(986103)));

        // The default graph. Other tests' rows are uncommitted and so
        // invisible; anything committed there is removed for this
        // transaction only.
        Spi::run("DELETE FROM pgrdf._pgrdf_quads WHERE graph_id = 0").unwrap();
        fill(0, &small_triples());
        out.push(("default", read(0)));

        new_graph(986104);
        fill(986104, &small_triples());
        put(
            986104,
            I("urn:wg:a"),
            I("urn:wg:p"),
            I("urn:wg:derived"),
            true,
        );
        put(986104, I("urn:wg:a"), I("urn:wg:p"), I("urn:wg:b"), true);
        put(986104, B("k"), I("urn:wg:p"), I("urn:wg:derived"), true);
        out.push(("inferred", read(986104)));

        new_graph(986105);
        fill(986105, &bnode_triples());
        fill(986105, &ground_triples()[3..]);
        Spi::run("SELECT pgrdf.lock_graph(986105, 'pinned')").unwrap();
        let first = read(986105);
        let again = read(986105);
        assert_eq!(first, again, "a locked graph reads the same twice");
        out.push(("locked", first));

        // A row whose subject is a literal: never exported, never
        // digested.
        new_graph(986106);
        fill(986106, &small_triples());
        put(
            986106,
            L("literal subject"),
            I("urn:wg:p"),
            I("urn:wg:b"),
            false,
        );
        out.push(("residue", read(986106)));

        out
    }

    /// One pinned fixture: name, `graph_digest`, `structural_digest`, the
    /// manifest's bytes / identity / structure digests, its asserted and
    /// inferred counts, and the full `export_graph` output.
    type Pinned = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        i64,
        i64,
        &'static [&'static str],
    );

    /// What 0.6.40 answers for each fixture, recorded from that code
    /// before the whole-graph functions were changed. These values are the
    /// acceptance oracle: a graph that holds no repeated triple must keep
    /// reading exactly this, byte for byte. Never regenerate them from the
    /// code under test.
    const PINNED: &[Pinned] = &[
        (
            "ground",
            "29dfff2905a2653ad83fb507e81755506c2bee0d83f6b3aea385064d2f0ceef5",
            "896b4bdc14bfa30f055eda36e8bd49341a9f469d1252037314ec471549a0c5ba",
            "29dfff2905a2653ad83fb507e81755506c2bee0d83f6b3aea385064d2f0ceef5",
            "29dfff2905a2653ad83fb507e81755506c2bee0d83f6b3aea385064d2f0ceef5",
            "896b4bdc14bfa30f055eda36e8bd49341a9f469d1252037314ec471549a0c5ba",
            18,
            0,
            &[
                "<urn:wg:s10> <urn:wg:p> <urn:wg:o1> .",
                "<urn:wg:s1> <urn:wg:p2> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"\" .",
                "<urn:wg:s1> <urn:wg:p> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<urn:wg:s1> <urn:wg:p> \"bare\" .",
                "<urn:wg:s1> <urn:wg:p> \"x y\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en-GB .",
                "<urn:wg:s1> <urn:wg:p> \"x\"^^<urn:wg:dt> .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o10> .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o1> .",
                "<urn:wg:s2> <urn:wg:p> \"a\\\" .\" .",
                "<urn:wg:s2> <urn:wg:p> \"ctl\u{1}and\u{7f}del\" .",
                "<urn:wg:s2> <urn:wg:p> \"héllo wörld — 日本語 🎉\" .",
                "<urn:wg:s2> <urn:wg:p> \"line1\\nline2\\ttab\\rcr \\\"quoted\\\" back\\\\slash\" .",
                "<urn:wg:s2> <urn:wg:q> \"été\"@fr .",
                "<urn:wg:ü> <urn:wg:p> <urn:wg:日本> .",
            ],
        ),
        (
            "bnodes",
            "5e944bb9d0ce344fd101322259e5756ae77b428e8e488ef8360c114178d95f80",
            "2e3118ad5f8766fc5ce9572733d55e2aafe4009c085fe27de6d01d9c6e9d762b",
            "e9b5adb12ddf256e6a9024bdf864b8d54879589b519ba98f9b725ff428c186f7",
            "5e944bb9d0ce344fd101322259e5756ae77b428e8e488ef8360c114178d95f80",
            "2e3118ad5f8766fc5ce9572733d55e2aafe4009c085fe27de6d01d9c6e9d762b",
            18,
            0,
            &[
                "<urn:wg:s1> <urn:wg:p> \"lit\" .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o> .",
                "<urn:wg:s1> <urn:wg:p> _:b1 .",
                "<urn:wg:s1> <urn:wg:q> \"z\" .",
                "<urn:wg:s2> <urn:wg:q> _:sh .",
                "<urn:wg:s3> <urn:wg:q> _:sh .",
                "_:b1 <urn:wg:p> \"t\\tab\" .",
                "_:b1 <urn:wg:p> <urn:wg:o> .",
                "_:b10 <urn:wg:p> <urn:wg:o> .",
                "_:b2 <urn:wg:p> _:b3 .",
                "_:b3 <urn:wg:p> _:b2 .",
                "_:c1 <urn:wg:p> \"marked\" .",
                "_:c1 <urn:wg:q> _:c2 .",
                "_:c2 <urn:wg:q> _:c3 .",
                "_:c3 <urn:wg:q> _:c4 .",
                "_:c4 <urn:wg:q> _:c1 .",
                "_:lone <urn:wg:p> _:lone .",
                "_:sh <urn:wg:p> \"v\"@en .",
            ],
        ),
        (
            "empty",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "0e7d07ee6ebef2a84399f1e308a9f4ab5afa93a21400c792ac85fba53456b145",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "0e7d07ee6ebef2a84399f1e308a9f4ab5afa93a21400c792ac85fba53456b145",
            0,
            0,
            &[],
        ),
        (
            "default",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            "87f00edf8b048040a71659c243de762d6b11cb56a149905b7178a79ccfc339b2",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            4,
            0,
            &[
                "<urn:wg:a> <urn:wg:p> \"one\" .",
                "<urn:wg:a> <urn:wg:p> <urn:wg:b> .",
                "<urn:wg:a> <urn:wg:p> _:k .",
                "_:k <urn:wg:p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            ],
        ),
        (
            "inferred",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            "87f00edf8b048040a71659c243de762d6b11cb56a149905b7178a79ccfc339b2",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            4,
            3,
            &[
                "<urn:wg:a> <urn:wg:p> \"one\" .",
                "<urn:wg:a> <urn:wg:p> <urn:wg:b> .",
                "<urn:wg:a> <urn:wg:p> _:k .",
                "_:k <urn:wg:p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            ],
        ),
        (
            "locked",
            "aa29e8bcdd392335be51c3f66ecc05c65ab976c5a5641269d708a1ce0f03d458",
            "8111dcafbbece2b29b97e0ee6b80c2cc2adb0cdb8cf27df4e8e2be132a7f6fae",
            "38afff29761e1e52a33d8d5592d08e79433b44cffd8c776193f7465710ee021d",
            "aa29e8bcdd392335be51c3f66ecc05c65ab976c5a5641269d708a1ce0f03d458",
            "8111dcafbbece2b29b97e0ee6b80c2cc2adb0cdb8cf27df4e8e2be132a7f6fae",
            33,
            0,
            &[
                "<urn:wg:s1> <urn:wg:p2> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"\" .",
                "<urn:wg:s1> <urn:wg:p> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<urn:wg:s1> <urn:wg:p> \"bare\" .",
                "<urn:wg:s1> <urn:wg:p> \"lit\" .",
                "<urn:wg:s1> <urn:wg:p> \"x y\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en-GB .",
                "<urn:wg:s1> <urn:wg:p> \"x\"^^<urn:wg:dt> .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o> .",
                "<urn:wg:s1> <urn:wg:p> _:b1 .",
                "<urn:wg:s1> <urn:wg:q> \"z\" .",
                "<urn:wg:s2> <urn:wg:p> \"a\\\" .\" .",
                "<urn:wg:s2> <urn:wg:p> \"ctl\u{1}and\u{7f}del\" .",
                "<urn:wg:s2> <urn:wg:p> \"héllo wörld — 日本語 🎉\" .",
                "<urn:wg:s2> <urn:wg:p> \"line1\\nline2\\ttab\\rcr \\\"quoted\\\" back\\\\slash\" .",
                "<urn:wg:s2> <urn:wg:q> \"été\"@fr .",
                "<urn:wg:s2> <urn:wg:q> _:sh .",
                "<urn:wg:s3> <urn:wg:q> _:sh .",
                "<urn:wg:ü> <urn:wg:p> <urn:wg:日本> .",
                "_:b1 <urn:wg:p> \"t\\tab\" .",
                "_:b1 <urn:wg:p> <urn:wg:o> .",
                "_:b10 <urn:wg:p> <urn:wg:o> .",
                "_:b2 <urn:wg:p> _:b3 .",
                "_:b3 <urn:wg:p> _:b2 .",
                "_:c1 <urn:wg:p> \"marked\" .",
                "_:c1 <urn:wg:q> _:c2 .",
                "_:c2 <urn:wg:q> _:c3 .",
                "_:c3 <urn:wg:q> _:c4 .",
                "_:c4 <urn:wg:q> _:c1 .",
                "_:lone <urn:wg:p> _:lone .",
                "_:sh <urn:wg:p> \"v\"@en .",
            ],
        ),
        (
            "residue",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            "87f00edf8b048040a71659c243de762d6b11cb56a149905b7178a79ccfc339b2",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            5,
            0,
            &[
                "<urn:wg:a> <urn:wg:p> \"one\" .",
                "<urn:wg:a> <urn:wg:p> <urn:wg:b> .",
                "<urn:wg:a> <urn:wg:p> _:k .",
                "_:k <urn:wg:p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            ],
        ),
    ];

    /// A graph without repeats reads byte-identically to 0.6.40: both
    /// digests, the three manifest digests, the counts and every export
    /// line, in order.
    #[pg_test]
    fn clean_graphs_read_as_before() {
        let got = build_all();
        assert_eq!(got.len(), PINNED.len());
        for ((name, r), pin) in got.iter().zip(PINNED) {
            assert_eq!(*name, pin.0);
            assert_eq!(r.graph_digest, pin.1, "{name}: graph_digest");
            assert_eq!(r.structural_digest, pin.2, "{name}: structural_digest");
            assert_eq!(r.manifest_bytes, pin.3, "{name}: manifest bytes digest");
            assert_eq!(r.manifest_identity, pin.4, "{name}: manifest identity");
            assert_eq!(r.manifest_structure, pin.5, "{name}: manifest structure");
            assert_eq!(r.asserted, pin.6, "{name}: asserted count");
            assert_eq!(r.inferred, pin.7, "{name}: inferred count");
            let want: Vec<String> = pin.8.iter().map(|l| l.to_string()).collect();
            assert_eq!(r.export, want, "{name}: export_graph");
        }
    }
}
