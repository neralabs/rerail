# The expansion program numbered: `python3 gen.py expand.lab > expand.rir` (a dev tool: words with
# labels and strings by text, numbered; the string table, the punctuators and the C++ rows listed).
import sys, re
lines = open(sys.argv[1]).read().splitlines()
strings = []
def s(text):
    if text not in strings: strings.append(text)
    return strings.index(text)
labels, words = {}, []
for l in lines:
    if l.startswith(';'): words.append(('c', l)); continue
    m = re.match(r'^(\w+):\s*(.*)$', l)
    body = l.strip()
    if m: labels[m.group(1)] = sum(1 for w in words if w[0] == 'w'); body = m.group(2)
    if body: words.append(('w', body))
out, pc = [], 0
for k, w in words:
    if k == 'c': out.append('  ' + w); continue
    code, _, comment = w.partition(';')
    code = re.sub(r'@(\w+)', lambda m: '@%04d' % labels[m.group(1)], code.strip())
    code = re.sub(r'"([^"]*)"', lambda m: 's%d' % s(m.group(1)), code)
    out.append(('  %04d %s' % (pc, code)) + (('  ;' + comment) if comment else ''))
    pc += 1
ops = ['<<=','>>=','...','->*','<=>','->','++','--','<<','>>','<=','>=','==','!=','&&','||','*=','/=','%=','+=','-=','&=','^=','|=','::','.*','<:',':>','<%','%>','%:','%:%:','##','#'] + list('()[]{};,~?:.=!+-*/%^&|<>')
cpp_only = {'->*','<=>','.*'}
oprows = ['  ppop s%d%s' % (s(o), '  ; C++ only' if o in cpp_only else '') for o in ops]
head = '''program - ir=7 grammar=none
; Decision 0348: a translation unit's preprocessing, as preprocessing words (the compiler's
; `pp_words`, which says what each word does). The lean preprocessor's `Pp` is its reference.
; A line ending `; C++ only` is left out of C's program (`expand::program`). Generated from a
; labeled text, the words numbered and their strings listed.
  mode set=0 query=0
'''
rows = ['  string %d "%s"' % (i, t) for i, t in enumerate(strings)]
print(head + '\n'.join(rows + oprows) + '\n  ppsuffix  ; C++ only\n  ppcpp  ; C++ only\n' + '\n'.join(out))
