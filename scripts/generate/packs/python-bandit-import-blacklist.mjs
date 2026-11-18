// Writes the Python pack's 13 ports of Bandit 1.9.4's import blacklist (bandit/blacklists/imports.py, B401 to B415):
// packs/python/rules/bandit-b4*-import-*.json, and packs/python/candidates/rules/<id>.json for a port marked
// `candidate` (listed in scripts/generate/README.md; edit this script, not the JSON). Every port is one matcher built
// from its blacklist entry's qualnames; only the description, the message, the candidate flag and B406's declared
// saxutils divergence differ. It reads no tools; run offline by a maintainer, never in CI; the output is deterministic,
// and test/scripts/generate/python-bandit-import-blacklist.test.mjs fails when a committed document differs from it.
//
// Usage (from the repository root): node scripts/generate/packs/python-bandit-import-blacklist.mjs
//
// Bandit reports an import statement when one of its names starts with a qualname, as a string. It reads every name of
// the statement; a from-import's name is its module joined to each imported name, with the relative dots dropped, so
// `from . import telnetlib` is `telnetlib` and `from xml.etree import ElementTree` is `xml.etree.ElementTree`. The
// matcher has three alternatives: an `import` naming a qualname, a from-import whose module starts with one, and, for
// each qualname's parent module (none for a top-level name), a from-import of that parent that imports the rest.
//
// A document keeps its committed revision when nothing else changes; one that changes gets the next revision, a
// candidate's too; a document with no committed version starts at revision 1. A port that moves between rules/ and
// candidates/rules/ leaves no copy behind.
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const PACK_DIRECTORY = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "packs", "python");

/**
 * B406's declared divergence (packs/python/README.md): an import of only xml.sax.saxutils, which holds XML writing and
 * escaping helpers and no parser, is not reported. `import xml.sax.saxutils` still is, since it binds xml.sax too.
 */
const SAXUTILS_ONLY = {
  any: [
    { pattern: "from xml.sax.saxutils import $$$NAMES" },
    {
      kind: "import_from_statement",
      all: [
        { has: { field: "module_name", regex: "^xml\\s*\\.\\s*sax$" } },
        { has: { any: [{ kind: "dotted_name" }, { kind: "aliased_import" }], follows: { stopBy: "end", regex: "^import$" } } },
        {
          not: {
            has: {
              any: [{ kind: "dotted_name" }, { kind: "aliased_import" }],
              follows: { stopBy: "end", regex: "^import$" },
              not: { regex: "^saxutils(?:\\s|$)" },
            },
          },
        },
      ],
    },
  ],
};

/**
 * Bandit 1.9.4's import blacklist, in its order. `qualnames` are the entry's, in Bandit's order; `name` is the entry's
 * name, which gives the rule id (`bandit-<code>-<name>` with dashes); `candidate` puts the port under candidates/rules/.
 * B410 (lxml) and B414 (pycryptodome) are not in Bandit 1.9.4's blacklist.
 */
export const BLACKLIST = [
  {
    code: "B401",
    name: "import_telnetlib",
    qualnames: ["telnetlib"],
    description: "Remove imports of telnetlib; Telnet sends credentials and data unencrypted, so use SSH or another encrypted protocol.",
    message: "Remove this telnetlib import and use SSH or another encrypted protocol instead.",
  },
  {
    code: "B402",
    name: "import_ftplib",
    qualnames: ["ftplib"],
    description: "Remove imports of ftplib; FTP sends credentials and data unencrypted, so use SFTP, SCP, or another encrypted protocol.",
    message: "Remove this ftplib import and use SFTP, SCP, or another encrypted protocol instead.",
  },
  {
    code: "B403",
    name: "import_pickle",
    candidate: true,
    qualnames: ["pickle", "cPickle", "dill", "shelve"],
    description: "Avoid importing pickle, cPickle, dill, or shelve; deserializing untrusted data with them can execute arbitrary code.",
    message: "Replace this import with json or another data-only format if the data it loads could be untrusted.",
  },
  {
    code: "B404",
    name: "import_subprocess",
    candidate: true,
    qualnames: ["subprocess"],
    description: "Avoid importing subprocess unless you need it; running external commands risks command injection when arguments come from untrusted input.",
    message: "Consider possible security implications associated with the subprocess module.",
  },
  {
    code: "B405",
    name: "import_xml_etree",
    qualnames: ["xml.etree.cElementTree", "xml.etree.ElementTree"],
    description: "Avoid importing xml.etree.ElementTree or cElementTree; use defusedxml instead, since the standard parsers are open to XML attacks on untrusted input.",
    message: "ElementTree XML parsing may be vulnerable to XML attacks; use defusedxml or defuse the standard library.",
  },
  {
    code: "B406",
    name: "import_xml_sax",
    qualnames: ["xml.sax"],
    except: SAXUTILS_ONLY,
    description: "Avoid parsing XML with xml.sax (importing only its saxutils helpers is fine); it is open to XML attacks, so use defusedxml.",
    message: "xml.sax parsing may be vulnerable to XML attacks; use defusedxml or defuse the standard library.",
  },
  {
    code: "B407",
    name: "import_xml_expat",
    qualnames: ["xml.dom.expatbuilder"],
    description: "Avoid importing xml.dom.expatbuilder to parse XML; it is open to XML attacks, so use defusedxml.",
    message: "xml.dom.expatbuilder parsing may be vulnerable to XML attacks; use defusedxml or defuse the standard library.",
  },
  {
    code: "B408",
    name: "import_xml_minidom",
    qualnames: ["xml.dom.minidom"],
    description: "Avoid importing xml.dom.minidom to parse XML; it is open to XML attacks, so use defusedxml.",
    message: "xml.dom.minidom parsing may be vulnerable to XML attacks; use defusedxml or defuse the standard library.",
  },
  {
    code: "B409",
    name: "import_xml_pulldom",
    qualnames: ["xml.dom.pulldom"],
    description: "Avoid importing xml.dom.pulldom to parse XML; it is open to XML attacks, so use defusedxml.",
    message: "xml.dom.pulldom parsing may be vulnerable to XML attacks; use defusedxml or defuse the standard library.",
  },
  {
    code: "B411",
    name: "import_xmlrpclib",
    qualnames: ["xmlrpc"],
    description: "Avoid importing xmlrpc unprotected; its XML parsing is open to XML attacks unless patched with defusedxml.",
    message: "xmlrpc parsing may be vulnerable to XML attacks; use defusedxml.xmlrpc.monkey_patch() to mitigate XML vulnerabilities.",
  },
  {
    code: "B412",
    name: "import_httpoxy",
    candidate: true,
    qualnames: ["wsgiref.handlers.CGIHandler", "twisted.web.twcgi.CGIScript", "twisted.web.twcgi.CGIDirectory"],
    description: "Avoid importing the CGI handlers from wsgiref and twisted.web; CGI handlers are exposed to the httpoxy proxy-header attack.",
    message: "Serve the app through a WSGI server instead of this CGI handler; CGI handlers are open to the httpoxy proxy-header attack.",
  },
  {
    code: "B413",
    name: "import_pycrypto",
    qualnames: ["Crypto.Cipher", "Crypto.Hash", "Crypto.IO", "Crypto.Protocol", "Crypto.PublicKey", "Crypto.Random", "Crypto.Signature", "Crypto.Util"],
    description: "Avoid importing the pyCrypto package; it is unmaintained and has known vulnerabilities, so use the cryptography package instead.",
    message: "The pyCrypto library is deprecated and unmaintained; consider using pyca/cryptography.",
  },
  {
    code: "B415",
    name: "import_pyghmi",
    qualnames: ["pyghmi"],
    description: "Avoid importing pyghmi; IPMI is an insecure protocol, so prefer an encrypted management protocol.",
    message: "Replace this pyghmi import with a client for an encrypted management protocol such as Redfish over HTTPS; IPMI is insecure.",
  },
];

/** The rule id of a blacklist entry: `bandit-b401-import-telnetlib`. */
export const ruleId = entry => `bandit-${entry.code.toLowerCase()}-${entry.name.replaceAll("_", "-")}`;

/** The document's path relative to the pack: under candidates/ for a candidate. */
export const documentPath = entry => join(...(entry.candidate ? ["candidates"] : []), "rules", `${ruleId(entry)}.json`);

/** A dotted name as source text spells it: tree-sitter keeps any whitespace around each dot in the node's text. */
const dotted = name => name.split(".").join("\\s*\\.\\s*");
const alternatives = names => (names.length === 1 ? names[0] : `(?:${names.join("|")})`);
const IMPORTED_NAME = [{ kind: "dotted_name" }, { kind: "aliased_import" }];

/** The qualnames grouped by parent module, in first-appearance order: `xml.etree.ElementTree` is `xml.etree` → `ElementTree`. */
function byParent(qualnames) {
  const groups = new Map();
  for (const qualname of qualnames) {
    const dot = qualname.lastIndexOf(".");
    const parent = dot === -1 ? "" : qualname.slice(0, dot);
    groups.set(parent, [...(groups.get(parent) ?? []), qualname.slice(dot + 1)]);
  }
  return [...groups];
}

function matcher({ qualnames, except }) {
  const names = alternatives(qualnames.map(dotted));
  const rule = {
    any: [
      { kind: "import_statement", has: { any: IMPORTED_NAME, regex: `^${names}` } },
      { kind: "import_from_statement", has: { field: "module_name", regex: `^[.\\s]*${names}` } },
      ...byParent(qualnames).map(([parent, leaves]) => ({
        kind: "import_from_statement",
        all: [
          // A top-level name has no parent: its from-import's module is only dots (`from . import telnetlib`).
          { has: { field: "module_name", regex: parent === "" ? "^[.\\s]+$" : `^[.\\s]*${dotted(parent)}$` } },
          { has: { any: IMPORTED_NAME, regex: `^${alternatives(leaves)}`, follows: { stopBy: "end", regex: "^import$" } } },
        ],
      })),
    ],
  };
  if (except !== undefined) rule.not = except;
  return rule;
}

/** The document's content, all but its format, id and revision. */
export function content(entry) {
  return {
    mode: "steer",
    description: entry.description,
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [
      {
        id: entry.code,
        forbid: { ast: { language: "python", rule: matcher(entry) } },
        violation: { code: entry.code, anchor: "witness.span", message: entry.message, suppress: { nosec: { codes: [entry.code] } } },
      },
    ],
  };
}

/** The document's text, given the committed document at its path (or undefined): the revision follows the file header's rule. */
export function render(entry, committed) {
  const body = content(entry);
  const unchanged = committed !== undefined && JSON.stringify({ mode: committed.mode, description: committed.description, scope: committed.scope, clauses: committed.clauses }) === JSON.stringify(body);
  const revision = committed === undefined ? 1 : unchanged ? committed.revision : committed.revision + 1;
  return `${JSON.stringify({ format: "rerail.policy/v1", id: ruleId(entry), revision, ...body }, null, 2)}\n`;
}

function write() {
  for (const entry of BLACKLIST) {
    const path = join(PACK_DIRECTORY, documentPath(entry));
    const other = join(PACK_DIRECTORY, documentPath({ ...entry, candidate: !entry.candidate }));
    // A port that moved keeps its revision line: the committed document is wherever it was.
    const previous = [path, other].find(candidate => existsSync(candidate));
    const committed = previous === undefined ? undefined : JSON.parse(readFileSync(previous, "utf8"));
    writeFileSync(path, render(entry, committed));
    rmSync(other, { force: true });
  }
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) write();
