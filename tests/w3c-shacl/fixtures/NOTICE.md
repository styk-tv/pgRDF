# NOTICE — vendored W3C SHACL test fixtures

The files under `core/` and `sparql/` are copied from, or derived from,
the W3C SHACL test suite. They are not pgRDF's own work and are not
covered by pgRDF's license; the W3C terms below apply to them.

## Source

| | |
|---|---|
| Work | SHACL Test Suite and Implementation Report — <https://w3c.github.io/data-shapes/data-shapes-test-suite/> |
| Repository | <https://github.com/w3c/data-shapes>, branch `gh-pages` |
| Commit | `eacdef4528d66ee116f0ac9118d968c5717c7233` |
| Directories | `data-shapes-test-suite/tests/core/` (→ `core/`) and `data-shapes-test-suite/tests/sparql/node/` (→ `sparql/`) |
| Retrieved | 2026-11-24 |

At that commit the files under `tests/core/` were last changed upstream
on 2019-03-31.

## What is in this directory

| File | What it is |
|---|---|
| `<name>.w3c.ttl` | The W3C test file, **unmodified** — byte-identical to the file at the commit above, line endings included. |
| `<name>.ttl` | The same file with exactly two top-level statements removed: the `<>` `mf:Manifest` statement and the `<test>` `sht:Validate` entry (with the expected report nested in it). Every other statement is kept as it is. This is the only modification. In `core/` the line endings are kept; in `sparql/` the split uses LF. |
| `<name>.expected.json` | Written for pgRDF's harness: `{"conforms":…}`, the `sh:conforms` value read from the entry's `mf:result` in the W3C file. Not a W3C file. |

The manifest is removed because the suite roots it at the empty relative
IRI `<>`, which needs a base IRI to parse; the validator does not use it.

Fixture names map to upstream paths as follows:

| Fixture prefix | Upstream directory | Example |
|---|---|---|
| `node-` | `tests/core/node/` | `node-and-001` ← `node/and-001.ttl` |
| `prop-` | `tests/core/property/` | `prop-datatype-001` ← `property/datatype-001.ttl` |
| `path-` | `tests/core/path/` | `path-inverse-001` ← `path/path-inverse-001.ttl` |
| `target-` | `tests/core/targets/` | `target-targetClass-001` ← `targets/targetClass-001.ttl` |
| `misc-` | `tests/core/misc/` | `misc-severity-001` ← `misc/severity-001.ttl` |
| `complex-` | `tests/core/complex/` | `complex-personexample` ← `complex/personexample.ttl` |
| `node-sparql-` (in `sparql/`) | `tests/sparql/node/` | `node-sparql-001` ← `sparql/node/sparql-001.ttl` |

All `*.w3c.ttl` files here — the 25 Core fixtures and the one
SHACL-SPARQL fixture vendored before this notice was written, and the
31 Core fixtures added with it — were compared with the commit above and
are byte-identical to it.

## License

The upstream repository states, in its `LICENSE.md`:

> All documents in this Repository are licensed by contributors under the
> [W3C Software and Document License](http://www.w3.org/Consortium/Legal/copyright-software).

The test files carry no notice of their own, so the W3C short notice is
given here for them:

> SHACL Test Suite: https://github.com/w3c/data-shapes
>
> Copyright © 2015-2019 World Wide Web Consortium. All Rights Reserved.
> This work is distributed under the W3C® Software and Document License
> [1] in the hope that it will be useful, but WITHOUT ANY WARRANTY;
> without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
> PARTICULAR PURPOSE.
>
> [1] https://www.w3.org/Consortium/Legal/copyright-software

(2015-2019 is the span of upstream commits to the suite's `tests/`
directory at the commit above.)

Notice of modification, as the license requires:

> This software or document includes material copied from or derived
> from SHACL Test Suite and Implementation Report,
> https://w3c.github.io/data-shapes/data-shapes-test-suite/.
> Copyright © 2015-2019 World Wide Web Consortium.
> https://www.w3.org/copyright/software-license-2023/

The modification is the manifest removal described above, in the
`<name>.ttl` files only.

Full text of the license notice (W3C Software and Document License, 2023
version, <https://www.w3.org/copyright/software-license-2023/>):

> This work is being provided by the copyright holders under the
> following license.
>
> **License**
>
> By obtaining and/or copying this work, you (the licensee) agree that
> you have read, understood, and will comply with the following terms and
> conditions.
>
> Permission to copy, modify, and distribute this work, with or without
> modification, for any purpose and without fee or royalty is hereby
> granted, provided that you include the following on ALL copies of the
> work or portions thereof, including modifications:
>
> - The full text of this NOTICE in a location viewable to users of the
>   redistributed or derivative work.
> - Any pre-existing intellectual property disclaimers, notices, or terms
>   and conditions. If none exist, the W3C software and document short
>   notice should be included.
> - Notice of any changes or modifications, through a copyright statement
>   on the new code or document such as "This software or document
>   includes material copied from or derived from [title and URI of the
>   W3C document]. Copyright © [$year-of-document] World Wide Web
>   Consortium. https://www.w3.org/copyright/software-license-2023/"
>
> **Disclaimers**
>
> THIS WORK IS PROVIDED "AS IS," AND COPYRIGHT HOLDERS MAKE NO
> REPRESENTATIONS OR WARRANTIES, EXPRESS OR IMPLIED, INCLUDING BUT NOT
> LIMITED TO, WARRANTIES OF MERCHANTABILITY OR FITNESS FOR ANY PARTICULAR
> PURPOSE OR THAT THE USE OF THE SOFTWARE OR DOCUMENT WILL NOT INFRINGE
> ANY THIRD PARTY PATENTS, COPYRIGHTS, TRADEMARKS OR OTHER RIGHTS.
>
> COPYRIGHT HOLDERS WILL NOT BE LIABLE FOR ANY DIRECT, INDIRECT, SPECIAL
> OR CONSEQUENTIAL DAMAGES ARISING OUT OF ANY USE OF THE SOFTWARE OR
> DOCUMENT.
>
> The name and trademarks of copyright holders may NOT be used in
> advertising or publicity pertaining to the work without specific,
> written prior permission. Title to copyright in this work will at all
> times remain with copyright holders.
