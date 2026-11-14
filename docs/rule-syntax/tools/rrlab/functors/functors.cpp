// rrlab's Soufflé functors: core.rr's @functors, plus the rr_* helpers the lowering emits for text tests and
// captures. Built by rrlab.py (c++ -shared) and loaded with souffle -l rrlabfn. Symbols in, symbol or number out;
// every returned string is interned here so the pointer outlives the call.
//
// Approximations (README, "Functors"): semver ignores prerelease tags; nfkc is the
// identity; to_precision follows %g; tree_digest and unparse are stubs; ed25519_ok uses libsodium when it is found (else a stub) (rrlab warns when a rule uses them).
#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cwctype>
#include <locale>
#include <map>
#include <mutex>
#include <set>
#include <sstream>
#include <string>
#include <vector>

using std::string;
static std::mutex mu;

static const char* intern(const string& s) {
    static std::set<string> pool;
    return pool.insert(s).first->c_str();
}

// Regexes, globs, word and lower are in src/rx.rs (the regex crate), linked into this library.
// rr_cap(re, s, i): group i of re's whole match of s; rr_capok: 1 when that group took part.
// ---- globs (core.rr, text tests) ---------------------------------------------------------------------------------
static string lower_s(const string& s);
extern "C" int32_t rr_prefix(const char* p, const char* s) { return strncmp(p, s, strlen(p)) == 0; }
extern "C" int32_t rr_suffix(const char* p, const char* s) {
    size_t a = strlen(p), b = strlen(s);
    return a <= b && memcmp(s + b - a, p, a) == 0;
}
extern "C" int32_t rr_contains(const char* p, const char* s) { return strstr(s, p) != nullptr; }
extern "C" int32_t rr_iprefix(const char* p, const char* s) { string a = lower_s(p), b = lower_s(s); return b.rfind(a, 0) == 0; }
extern "C" int32_t rr_isuffix(const char* p, const char* s) {
    string a = lower_s(p), b = lower_s(s);
    return a.size() <= b.size() && b.compare(b.size() - a.size(), a.size(), a) == 0;
}
extern "C" int32_t rr_icontains(const char* p, const char* s) { return lower_s(s).find(lower_s(p)) != string::npos; }
// word(w, s): w's words occur in s as whole words, any non-word run between them.
// split_dot helpers: q = m.x at the last dot.
extern "C" int32_t rr_has_dot(const char* q) { return strchr(q, '.') != nullptr; }
extern "C" const char* rr_dot_init(const char* q) { string s(q); size_t i = s.rfind('.'); return intern(i == string::npos ? "" : s.substr(0, i)); }
extern "C" const char* rr_dot_last(const char* q) { string s(q); size_t i = s.rfind('.'); return intern(i == string::npos ? s : s.substr(i + 1)); }

// ---- names and paths ---------------------------------------------------------------------------------------------
extern "C" const char* dot(const char* a, const char* b) { return intern(*a ? string(a) + "." + b : string(b)); }
extern "C" const char* qname(const char* m, const char* x) { return intern(string(m) + "#" + x); }
extern "C" const char* pkg_up(const char* q, int32_t k) {
    string s(q);
    for (int i = 0; i < k; i++) { size_t j = s.rfind('.'); if (j == string::npos) return ""; s = s.substr(0, j); }
    return intern(s);
}
extern "C" const char* head(const char* q) { string s(q); return intern(s.substr(0, s.find('.'))); }
extern "C" const char* dir(const char* p) { string s(p); size_t i = s.rfind('/'); return intern(i == string::npos ? "" : s.substr(0, i)); }
extern "C" const char* join(const char* d, const char* rel) {
    string all = *d ? string(d) + "/" + rel : string(rel);
    std::vector<string> parts;
    std::stringstream ss(all);
    string seg;
    while (std::getline(ss, seg, '/')) {
        if (seg.empty() || seg == ".") continue;
        if (seg == ".." && !parts.empty() && parts.back() != "..") parts.pop_back();
        else parts.push_back(seg);
    }
    string o;
    for (auto& p : parts) { if (!o.empty()) o += "/"; o += p; }
    return intern(o);
}
extern "C" const char* glob_sub(const char* pat, const char* to, const char* s) {
    string p(pat), t(to), x(s);
    size_t star = p.find('*');
    if (star == string::npos) return intern(p == x ? t : "");
    string pre = p.substr(0, star), post = p.substr(star + 1);
    if (x.size() < pre.size() + post.size() || x.compare(0, pre.size(), pre) || x.compare(x.size() - post.size(), post.size(), post)) return "";
    string mid = x.substr(pre.size(), x.size() - pre.size() - post.size());
    size_t ts = t.find('*');
    return intern(ts == string::npos ? t : t.substr(0, ts) + mid + t.substr(ts + 1));
}
extern "C" const char* package_of(const char* spec) { return spec; }

// ---- text --------------------------------------------------------------------------------------------------------
static std::u32string u32(const string& s) {
    std::u32string o;
    for (size_t i = 0; i < s.size();) {
        unsigned char c = s[i];
        int n = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
        char32_t cp = n == 1 ? c : n == 2 ? c & 0x1F : n == 3 ? c & 0x0F : c & 0x07;
        for (int k = 1; k < n && i + k < s.size(); k++) cp = (cp << 6) | (s[i + k] & 0x3F);
        o += cp;
        i += n;
    }
    return o;
}
static string u8(const std::u32string& s) {
    string o;
    for (char32_t c : s) {
        if (c < 0x80) o += (char)c;
        else if (c < 0x800) { o += (char)(0xC0 | (c >> 6)); o += (char)(0x80 | (c & 0x3F)); }
        else if (c < 0x10000) { o += (char)(0xE0 | (c >> 12)); o += (char)(0x80 | ((c >> 6) & 0x3F)); o += (char)(0x80 | (c & 0x3F)); }
        else { o += (char)(0xF0 | (c >> 18)); o += (char)(0x80 | ((c >> 12) & 0x3F)); o += (char)(0x80 | ((c >> 6) & 0x3F)); o += (char)(0x80 | (c & 0x3F)); }
    }
    return o;
}
static string lower_s(const string& s) {
    static bool init = (setlocale(LC_CTYPE, "en_US.UTF-8"), true);
    (void)init;
    std::u32string u = u32(s);
    for (auto& c : u) c = (char32_t)towlower((wint_t)c);
    return u8(u);
}
extern "C" const char* nfkc(const char* s) { return s; }
extern "C" const char* slice(const char* s, int32_t a, int32_t b) {
    std::u32string u = u32(s);
    int32_t n = (int32_t)u.size();
    if (a < 0) a = std::max(0, n + a);
    if (b < 0) b = std::max(0, n + b);
    a = std::min(a, n); b = std::min(b, n);
    return intern(a < b ? u8(u.substr(a, b - a)) : "");
}
static std::vector<string> words(const string& s, const string& seps) {
    std::vector<string> w;
    string cur;
    bool dash = seps.empty() || seps.find("dash") != string::npos, under = seps.empty() || seps.find("underscore") != string::npos;
    bool space = seps.empty() || seps.find("space") != string::npos;
    for (size_t i = 0; i < s.size(); i++) {
        char c = s[i];
        bool sep = (c == '-' && dash) || (c == '_' && under) || (c == ' ' && space) || (seps.empty() && c == '.');
        bool camel = seps.empty() && isupper((unsigned char)c) && !cur.empty() && islower((unsigned char)cur.back());
        if (sep || camel) { if (!cur.empty()) w.push_back(cur); cur.clear(); if (sep) continue; }
        cur += c;
    }
    if (!cur.empty()) w.push_back(cur);
    return w;
}
extern "C" const char* convert(const char* s, const char* to, const char* seps) {
    string t(to);
    auto ws = words(s, seps);
    auto low = [](string x) { for (auto& c : x) c = tolower((unsigned char)c); return x; };
    auto cap = [&](string x) { x = low(x); if (!x.empty()) x[0] = toupper((unsigned char)x[0]); return x; };
    string o;
    if (t == "lowerCase") { o = low(s); }
    else if (t == "upperCase") { o = s; for (auto& c : o) c = toupper((unsigned char)c); }
    else if (t == "capitalize") { o = s; if (!o.empty()) o[0] = toupper((unsigned char)o[0]); }
    else if (t == "camelCase" || t == "pascalCase") {
        for (size_t i = 0; i < ws.size(); i++) o += (i == 0 && t == "camelCase") ? low(ws[i]) : cap(ws[i]);
    } else if (t == "snakeCase" || t == "kebabCase") {
        for (size_t i = 0; i < ws.size(); i++) { if (i) o += t == "snakeCase" ? "_" : "-"; o += low(ws[i]); }
    } else o = s;
    return intern(o);
}
extern "C" const char* to_precision(const char* n, int32_t digits) {
    double v = atof(n);
    char buf[64];
    int e = v == 0 ? 0 : (int)floor(log10(fabs(v)));
    if (e < -6 || e >= digits) {
        snprintf(buf, sizeof buf, "%.*e", digits - 1, v);
        string s(buf);  // 1.23e+05 -> 1.23e+5
        size_t p = s.find('e');
        string mant = s.substr(0, p), ex = s.substr(p + 1);
        int ev = atoi(ex.c_str());
        return intern(mant + "e" + (ev < 0 ? "-" : "+") + std::to_string(abs(ev)));
    }
    snprintf(buf, sizeof buf, "%.*f", std::max(0, digits - 1 - e), v);
    return intern(buf);
}
extern "C" const char* json_str(const char* s) {
    std::u32string u = u32(s);
    string o = "\"";
    char buf[16];
    for (char32_t c : u) {
        if (c == '"') o += "\\\"";
        else if (c == '\\') o += "\\\\";
        else if (c == '\n') o += "\\n";
        else if (c == '\r') o += "\\r";
        else if (c == '\t') o += "\\t";
        else if (c == '\b') o += "\\b";
        else if (c == '\f') o += "\\f";
        else if (c < 0x20 || (c >= 0x7F && c < 0x10000)) { snprintf(buf, sizeof buf, "\\u%04x", (unsigned)c); o += buf; }
        else if (c >= 0x10000) {
            char32_t v = c - 0x10000;
            snprintf(buf, sizeof buf, "\\u%04x\\u%04x", (unsigned)(0xD800 + (v >> 10)), (unsigned)(0xDC00 + (v & 0x3FF)));
            o += buf;
        } else o += (char)c;
    }
    return intern(o + "\"");
}

// sha256 (FIPS 180-4).
extern "C" const char* sha256(const char* s) {
    static const uint32_t K[64] = {
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
        0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
        0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2};
    uint32_t h[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
    string m(s);
    uint64_t bits = (uint64_t)m.size() * 8;
    m += (char)0x80;
    while (m.size() % 64 != 56) m += (char)0;
    for (int i = 7; i >= 0; i--) m += (char)(bits >> (i * 8));
    auto rot = [](uint32_t x, int n) { return (x >> n) | (x << (32 - n)); };
    for (size_t off = 0; off < m.size(); off += 64) {
        uint32_t w[64];
        for (int i = 0; i < 16; i++)
            w[i] = (uint32_t)(unsigned char)m[off + 4 * i] << 24 | (uint32_t)(unsigned char)m[off + 4 * i + 1] << 16 |
                   (uint32_t)(unsigned char)m[off + 4 * i + 2] << 8 | (uint32_t)(unsigned char)m[off + 4 * i + 3];
        for (int i = 16; i < 64; i++) {
            uint32_t s0 = rot(w[i - 15], 7) ^ rot(w[i - 15], 18) ^ (w[i - 15] >> 3);
            uint32_t s1 = rot(w[i - 2], 17) ^ rot(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
        for (int i = 0; i < 64; i++) {
            uint32_t t1 = hh + (rot(e, 6) ^ rot(e, 11) ^ rot(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i];
            uint32_t t2 = (rot(a, 2) ^ rot(a, 13) ^ rot(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
            hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
        }
        h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
    }
    char buf[65];
    for (int i = 0; i < 8; i++) snprintf(buf + 8 * i, 9, "%08x", h[i]);
    return intern(buf);
}

// ---- semver (node-semver ranges, approximated) --------------------------------------------------------------------
// A range is a union of intervals over (major, minor, patch); prerelease tags are dropped. Supports ||, space-AND,
// comparators (<, <=, >, >=, =), ^, ~, x-ranges (1.x, 1.2.*, *), hyphen ranges and a leading v.
typedef std::vector<long> V;  // 3 parts
static const long INF = 1L << 40;
struct Iv { V lo; bool loInc; V hi; bool hiInc; };
static bool lt(const V& a, const V& b) { return a < b; }
static bool parse_partial(string s, V& v, int& n) {
    while (!s.empty() && (s[0] == 'v' || s[0] == '=' || s[0] == 'V')) s = s.substr(1);
    size_t dash = s.find_first_of("-+");
    if (dash != string::npos) s = s.substr(0, dash);
    v = {0, 0, 0}; n = 0;
    if (s.empty() || s == "*" || s == "x" || s == "X") return true;
    std::stringstream ss(s);
    string part;
    while (std::getline(ss, part, '.')) {
        if (n >= 3) return false;
        if (part == "x" || part == "X" || part == "*") break;
        if (part.empty() || !std::all_of(part.begin(), part.end(), ::isdigit)) return false;
        v[n++] = atol(part.c_str());
    }
    return true;
}
static V bump(V v, int n) { V o = v; if (n == 0) return {INF, 0, 0}; o[n - 1]++; for (int i = n; i < 3; i++) o[i] = 0; return o; }
static bool comparator(const string& c, Iv& iv) {
    iv = {{0, 0, 0}, true, {INF, 0, 0}, false};
    string op, rest = c;
    for (const char* o : {">=", "<=", ">", "<", "=", "^", "~"}) if (rest.rfind(o, 0) == 0) { op = o; rest = rest.substr(strlen(o)); break; }
    if (op == "~" && !rest.empty() && rest[0] == '>') rest = rest.substr(1);
    while (!rest.empty() && rest[0] == ' ') rest = rest.substr(1);
    V v; int n;
    if (!parse_partial(rest, v, n)) return false;
    if (op == "" || op == "=") { iv.lo = v; iv.hi = bump(v, n); if (n == 3) { iv.hi = v; iv.hiInc = true; } }
    else if (op == ">=") iv.lo = v;
    else if (op == ">") { if (n == 3) { iv.lo = v; iv.loInc = false; } else iv.lo = bump(v, n); }
    else if (op == "<") { iv.hi = v; iv.lo = {0, 0, 0}; }
    else if (op == "<=") { if (n == 3) { iv.hi = v; iv.hiInc = true; } else iv.hi = bump(v, n); }
    else if (op == "~") { iv.lo = v; iv.hi = bump(v, n >= 2 ? 2 : n); }
    else if (op == "^") {  // the first non-zero part given stays; ^0.0.3 is <0.0.4, ^0.2 is <0.3.0, ^1 is <2
        iv.lo = v;
        int k = (v[0] || n <= 1) ? 1 : (v[1] || n == 2) ? 2 : 3;
        iv.hi = bump(v, k);
    }
    return true;
}
static bool inter(Iv& a, const Iv& b) {
    if (lt(a.lo, b.lo) || (a.lo == b.lo && !b.loInc)) { a.lo = b.lo; a.loInc = b.loInc; }
    if (lt(b.hi, a.hi) || (a.hi == b.hi && !b.hiInc)) { a.hi = b.hi; a.hiInc = b.hiInc; }
    return lt(a.lo, a.hi) || (a.lo == a.hi && a.loInc && a.hiInc);
}
static bool parse_range(const string& r, std::vector<Iv>& out) {
    out.clear();
    string s = r;
    size_t p = 0;
    while (true) {
        size_t q = s.find("||", p);
        string alt = s.substr(p, q == string::npos ? string::npos : q - p);
        // tokens; join an operator with the version after it ("> 1.2" is ">1.2")
        std::vector<string> toks;
        std::stringstream ss(alt);
        string t;
        while (ss >> t) {
            if (!toks.empty() && (toks.back() == ">=" || toks.back() == "<=" || toks.back() == ">" || toks.back() == "<" ||
                                  toks.back() == "=" || toks.back() == "^" || toks.back() == "~")) toks.back() += t;
            else toks.push_back(t);
        }
        Iv acc = {{0, 0, 0}, true, {INF, 0, 0}, false};
        bool ok = true;
        if (toks.size() == 3 && toks[1] == "-") {
            V a, b; int na, nb;
            if (!parse_partial(toks[0], a, na) || !parse_partial(toks[2], b, nb)) return false;
            acc.lo = a;
            if (nb == 3) { acc.hi = b; acc.hiInc = true; } else acc.hi = bump(b, nb);
        } else {
            for (auto& c : toks) { Iv iv; if (!comparator(c, iv)) return false; if (!inter(acc, iv)) ok = false; }
        }
        if (ok) out.push_back(acc);
        if (q == string::npos) break;
        p = q + 2;
    }
    return true;
}
extern "C" int32_t semver_valid(const char* r) { std::vector<Iv> v; return parse_range(r, v) ? 1 : 0; }
// a ⊆ b: every interval of a is covered by the union of b's (b's intervals merged first).
extern "C" int32_t semver_subset(const char* a, const char* b) {
    std::vector<Iv> A, B;
    if (!parse_range(a, A) || !parse_range(b, B)) return 0;
    std::sort(B.begin(), B.end(), [](const Iv& x, const Iv& y) { return lt(x.lo, y.lo) || (x.lo == y.lo && x.loInc && !y.loInc); });
    for (auto& x : A) {
        V cur = x.lo; bool curInc = x.loInc;  // the next point still to cover
        bool covered = false;
        for (int pass = 0; pass < 2 * (int)B.size() + 1 && !covered; pass++) {
            bool moved = false;
            for (auto& y : B) {
                bool startsOk = lt(y.lo, cur) || (y.lo == cur && (y.loInc || !curInc));
                bool reaches = lt(cur, y.hi) || (cur == y.hi && y.hiInc && curInc);
                if (!startsOk || !reaches) continue;
                if (lt(x.hi, y.hi) || (x.hi == y.hi && (y.hiInc || !x.hiInc))) { covered = true; break; }
                if (lt(cur, y.hi) || (cur == y.hi && !curInc)) { cur = y.hi; curInc = !y.hiInc; moved = true; }
            }
            if (!moved) break;
        }
        if (!covered) return 0;
    }
    return 1;
}

// ---- places and stubs --------------------------------------------------------------------------------------------
// @place(path, line): a span id from 2^30 up; each new one is appended to $RRLAB_PLACES as id\tpath\tline.
extern "C" int32_t place(const char* path, int32_t line) {
    std::lock_guard<std::mutex> g(mu);
    static std::map<std::pair<string, int32_t>, int32_t> ids;
    auto key = std::make_pair(string(path), line);
    auto it = ids.find(key);
    if (it != ids.end()) return it->second;
    int32_t id = (1 << 30) + (int32_t)ids.size();
    ids[key] = id;
    if (const char* f = getenv("RRLAB_PLACES")) {
        if (FILE* fp = fopen(f, "a")) { fprintf(fp, "%d\t%s\t%d\n", id, path, line); fclose(fp); }
    }
    return id;
}
extern "C" const char* tree_digest(int32_t n) { return intern("node:" + std::to_string(n)); }
extern "C" const char* unparse(int32_t n) { return intern("<node " + std::to_string(n) + ">"); }
#ifdef RRLAB_SODIUM
#include <sodium.h>
// key: 64 hex digits; sig: base64, decoded as Python's base64.b64decode does by default (characters outside the
// alphabet are dropped, padding is not required to be exact). 1 when sig is key's Ed25519 signature of message.
extern "C" int32_t ed25519_ok(const char* key, const char* message, const char* sig) {
    if (sodium_init() < 0) return 0;
    unsigned char pk[32];
    size_t n = 0;
    if (strlen(key) != 64 || sodium_hex2bin(pk, 32, key, 64, nullptr, &n, nullptr) != 0 || n != 32) return 0;
    std::string b;
    for (const char* c = sig; *c; ++c)
        if (isalnum((unsigned char)*c) || *c == '+' || *c == '/') b += *c;
    std::string raw;
    int val = 0, bits = 0;
    for (char c : b) {
        int d = isupper((unsigned char)c) ? c - 'A' : islower((unsigned char)c) ? c - 'a' + 26
              : isdigit((unsigned char)c) ? c - '0' + 52 : c == '+' ? 62 : 63;
        val = (val << 6) | d; bits += 6;
        if (bits >= 8) { bits -= 8; raw += (char)((val >> bits) & 0xff); }
    }
    if (raw.size() != crypto_sign_BYTES) return 0;
    return crypto_sign_verify_detached((const unsigned char*)raw.data(), (const unsigned char*)message,
                                       strlen(message), pk) == 0 ? 1 : 0;
}
#else
extern "C" int32_t ed25519_ok(const char*, const char*, const char*) { return 0; }
#endif
