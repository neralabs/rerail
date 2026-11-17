/**
 * The Bandit import-blacklist ports (B401 to B415) must report what Bandit 1.9.4 reports. Bandit's blacklist
 * (bandit/core/blacklisting.py) checks every name of an `import` or `from ... import` statement, takes `import a.b as c`
 * by its module `a.b`, joins a `from` statement's module (dots of a relative import dropped) and each imported name
 * with a dot, and reports the statement when any such full name starts with a blacklisted qualified name, as a string
 * prefix: `import os, subprocess`, `import telnetlib3 as telnetlib` and `from . import pickle` are all reported.
 *
 * Each case is one source and the 1-based lines Bandit 1.9.4 reports on it, run as `bandit -t <id>` (one test, so a
 * statement another blacklist test would claim first still counts). B406's last six statements are the pack's declared
 * divergence: an import of only xml.sax.saxutils is not reported.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");

const evaluationSchema = z.object({
  violations: z.array(z.object({ span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

interface Case {
  readonly rule: string;
  readonly folder: readonly string[];
  readonly source: readonly string[];
  readonly reported: readonly number[];
}

const SHIPPED = ["rules"];
const CANDIDATE = ["candidates", "rules"];

const CASES: readonly Case[] = [
  {
    rule: "bandit-b401-import-telnetlib",
    folder: SHIPPED,
    source: [
      "import os, telnetlib",
      "import telnetlib3 as telnetlib",
      "import os as o, telnetlib as t",
      "import telnetlib",
      "import telnetlib as t",
      "from telnetlib import Telnet",
      "from telnetlib3 import Telnet",
      "from . import telnetlib",
      "from .telnetlib import Telnet",
      "from .. import os, telnetlib",
      "from os import (path,",
      "    telnetlib)",
      "import os",
      "import net.telnetlib",
      "from net import telnetlib",
      "from .net import telnetlib",
      "import os as telnetlib",
    ],
    reported: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
  },
  {
    rule: "bandit-b402-import-ftplib",
    folder: SHIPPED,
    source: [
      "import os, ftplib",
      "import ftplib2 as ftplib",
      "import sys as s, ftplib as f",
      "import ftplib",
      "from ftplib import FTP, FTP_TLS",
      "from . import ftplib",
      "import pyftplib",
      "from net import ftplib",
      "from os import path as ftplib",
    ],
    reported: [1, 2, 3, 4, 5, 6],
  },
  {
    rule: "bandit-b403-import-pickle",
    folder: CANDIDATE,
    source: [
      "import os, pickle",
      "import cPickle as pickle",
      "import pickleshare",
      "import json, dill as d",
      "from . import shelve",
      "import os, shelve",
      "from dill.source import getsource",
      "import pickle",
      "from pickle import loads",
      "import cloudpickle",
      "import _pickle",
      "from joblib import pickle",
      "import sys as pickle",
    ],
    reported: [1, 2, 3, 4, 5, 6, 7, 8, 9],
  },
  {
    rule: "bandit-b404-import-subprocess",
    folder: CANDIDATE,
    source: [
      "import os, subprocess",
      "import subprocess32 as subprocess",
      "from . import subprocess",
      "from .subprocess import run",
      "import sys, os as o, subprocess as sp",
      "from subprocess import (",
      "    PIPE,",
      "    run,",
      ")",
      "import subprocess",
      "from asyncio import subprocess",
      "import asyncio.subprocess",
      "from os import (path, sep)",
      "from os import path as subprocess",
    ],
    reported: [1, 2, 3, 4, 5, 6, 10],
  },
  {
    rule: "bandit-b405-import-xml-etree",
    folder: SHIPPED,
    source: [
      "import os, xml.etree.ElementTree",
      "import xml.etree.cElementTree as ET",
      "import sys, xml.etree.ElementTree as ET",
      "from xml.etree import ElementInclude, ElementTree",
      "from xml.etree import ElementInclude, cElementTree as ET",
      "from xml.etree.ElementTree import parse",
      "from .xml.etree import ElementTree",
      "import xml.etree.ElementTree",
      "import xml.etree",
      "from xml import etree",
      "from xml.etree import ElementInclude",
      "import lxml.etree",
      "from lxml.etree import fromstring",
      "from xml.etree import *",
    ],
    reported: [1, 2, 3, 4, 5, 6, 7, 8],
  },
  {
    rule: "bandit-b406-import-xml-sax",
    folder: SHIPPED,
    source: [
      "import os, xml.sax",
      "import xml.sax.handler as h",
      "from xml import dom, sax",
      "from xml.sax import saxutils, handler",
      "from xml.sax import handler, saxutils",
      "from xml.sax import (saxutils, handler)",
      "from xml.sax import *",
      "from xml.sax import make_parser",
      "import xml.sax",
      "from .xml import sax",
      "import xml.sax.saxutils",
      "import xml",
      "from xml import dom",
      "from xml.sax.saxutils import escape",
      "from xml.sax.saxutils import (escape,",
      "    quoteattr)",
      "from xml.sax import saxutils",
      "from xml.sax import (saxutils)",
      "from xml.sax import saxutils as su",
      "from xml.sax import (saxutils as su)",
    ],
    reported: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
  },
  {
    rule: "bandit-b407-import-xml-expat",
    folder: SHIPPED,
    source: [
      "import os, xml.dom.expatbuilder",
      "import xml.dom.expatbuilder as eb",
      "from xml.dom import Node, expatbuilder",
      "from xml.dom.expatbuilder import parse",
      "from .xml.dom import expatbuilder",
      "import xml.dom",
      "from xml import dom",
      "from xml.dom import Node",
    ],
    reported: [1, 2, 3, 4, 5],
  },
  {
    rule: "bandit-b408-import-xml-minidom",
    folder: SHIPPED,
    source: [
      "import sys, xml.dom.minidom",
      "import xml.dom.minidom as md",
      "from xml.dom import Node, minidom as md",
      "from xml.dom.minidom import parseString",
      "from ..xml.dom import minidom",
      "import xml.dom",
      "from xml.dom import Node",
    ],
    reported: [1, 2, 3, 4, 5],
  },
  {
    rule: "bandit-b409-import-xml-pulldom",
    folder: SHIPPED,
    source: [
      "import sys, xml.dom.pulldom",
      "import xml.dom.pulldom as pd",
      "from xml.dom import Node, pulldom",
      "from xml.dom.pulldom import parse",
      "from .xml.dom.pulldom import parse",
      "import xml.dom",
      "from xml.dom import Node",
    ],
    reported: [1, 2, 3, 4, 5],
  },
  {
    rule: "bandit-b411-import-xmlrpclib",
    folder: SHIPPED,
    source: [
      "import os, xmlrpc.client",
      "import xmlrpclib as x",
      "from xmlrpc.server import SimpleXMLRPCServer",
      "from . import xmlrpc",
      "import xmlrpc",
      "import defusedxml.xmlrpc",
      "from defusedxml import xmlrpc",
    ],
    reported: [1, 2, 3, 4, 5],
  },
  {
    rule: "bandit-b412-import-httpoxy",
    folder: CANDIDATE,
    source: [
      "import os, wsgiref.handlers.CGIHandler",
      "from wsgiref.handlers import BaseHandler, CGIHandler",
      "from wsgiref.handlers import CGIHandler as H",
      "from twisted.web.twcgi import CGIScript, CGIDirectory",
      "from twisted.web.twcgi import FilteredScript, CGIDirectory",
      "from wsgiref.handlers import CGIHandler",
      "from wsgiref.handlers import BaseHandler",
      "import wsgiref.handlers",
      "from wsgiref import handlers",
      "import twisted.web.twcgi",
      "from twisted.web import twcgi",
    ],
    reported: [1, 2, 3, 4, 5, 6],
  },
  {
    rule: "bandit-b413-import-pycrypto",
    folder: SHIPPED,
    source: [
      "import os, Crypto.Cipher.AES",
      "from Crypto import Random, Hash",
      "import Crypto.PublicKey.RSA as RSA",
      "from Crypto.Util.number import bytes_to_long",
      "from Crypto import os, Signature",
      "import Crypto.Cipher",
      "from Crypto.Cipher import AES",
      "from .Crypto import Cipher",
      "import Crypto",
      "from Crypto import os",
      "import Cryptodome.Cipher",
      "from Cryptodome.Cipher import AES",
    ],
    reported: [1, 2, 3, 4, 5, 6, 7, 8],
  },
  {
    rule: "bandit-b415-import-pyghmi",
    folder: SHIPPED,
    source: [
      "import os, pyghmi",
      "import pyghmi.ipmi.command as cmd",
      "from pyghmi.ipmi import command",
      "import pyghmix",
      "import pyghmi",
      "from vendor import pyghmi",
      "import os",
    ],
    reported: [1, 2, 3, 4, 5],
  },
];

/** The sorted 1-based lines `rule` reports in a file holding `source`. */
async function reportedLines(rule: string, folder: readonly string[], source: readonly string[]): Promise<number[]> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = await readFile(repositoryPath("packs", "python", ...folder, `${rule}.json`), "utf8");
  const compiled = runtime.compile(document, "json", capabilities);
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "app/module.py", source: `${source.join("\n")}\n` }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

describe("Bandit import-blacklist ports", () => {
  it.each(CASES.map(entry => [entry.rule, entry] as const))("%s reports what Bandit 1.9.4 reports", async (_, entry) => {
    expect(await reportedLines(entry.rule, entry.folder, entry.source)).toEqual(entry.reported);
  });
});
