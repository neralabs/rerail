import { writeFileSync } from 'node:fs';
const dir = new URL('.', import.meta.url).pathname;
const L = `(?:^|[\\s'"\`:])`; // left edge of a class name (start, space, quote, backtick, variant colon)
const R = `(?:$|[\\s'"\`])`; // right edge
const SRC = ['src/**/*.ts', 'src/**/*.tsx'];
const NOT_TESTS = ['**/*.test.ts', '**/*.test.tsx', 'src/types/database.ts'];
const rx = (pattern) => ({ regex: { pattern, source: 'file.content' } });
const clause = (id, forbid, code, message) => ({ id, forbid, violation: { code, anchor: 'witness.span', message } });
const draft = (name, description, scope, clauses, caught, passed, mode = 'steer') => {
  const d = {
    kind: 'code',
    rule: { format: 'rerail.policy/v1', id: `local:${name}`, revision: 1, mode, description, scope, clauses },
    cases: { caught, passed },
  };
  writeFileSync(`${dir}${name}.json`, JSON.stringify(d, null, 2));
};
const c = (path, content) => ({ path, content });

// 1. Colors only from tokens
const PALETTE = 'slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose';
const UTIL = 'bg|text|border(?:-[xytrblse])?|ring|ring-offset|fill|stroke|divide|outline|placeholder|decoration|from|via|to|caret|accent';
draft(
  'color-tokens-only',
  'Use the color tokens from the theme instead of hex, rgb or hsl values or Tailwind palette colors in app code; the token file itself is exempt.',
  { include: SRC, exclude: [...NOT_TESTS, 'src/theme/**'] },
  [
    clause('hex', rx(`(?:['"\`\\[]|:\\s*)#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\\b`), 'HEX_COLOR',
      'Replace this hex color with a token class (bg-card, text-muted-foreground, ...) or `color(scheme, name)` from src/theme/tokens.ts.'),
    clause('color-fn', rx(`\\b(?:rgba?|hsla?)\\(\\s*[0-9.]`), 'COLOR_FUNCTION',
      'Replace this color value with a token class or `color(scheme, name)` from src/theme/tokens.ts; new colors go in tokens.ts only after DESIGN.md approves them.'),
    clause('palette-class', rx(`${L}(?:${UTIL})-(?:(?:${PALETTE})-(?:50|[1-9]00|950)|white|black)\\b`), 'PALETTE_COLOR',
      'Use a token color class (foreground, primary-foreground, muted, border, attention...) instead of a Tailwind palette color.'),
    clause('named-color', rx(`\\b(?:color|backgroundColor|borderColor|tintColor|placeholderTextColor|fill|stroke)\\s*[:=]\\s*\\{?\\s*['"](?:white|black|red|green|blue|gray|grey|orange|yellow)['"]`), 'NAMED_COLOR',
      'Replace this named color with `color(scheme, name)` from src/theme/tokens.ts or a token class.'),
  ],
  [
    c('src/components/ui/button.tsx', "const v = { destructive: 'text-white' };\n"),
    c('src/app/a.tsx', "const v = { destructive: 'text-white' };\n"),
    c('src/features/b.tsx', 'export const s = { color: \'#9A5208\' };\n'),
    c('src/app/c.tsx', 'const k = <View className="bg-[#fff] p-4" />;\n'),
    c('src/app/d.tsx', "const k = { backgroundColor: 'rgba(0, 0, 0, 0.5)' };\n"),
    c('src/app/e.tsx', 'const k = <Text className="dark:text-red-500" />;\n'),
    c('src/app/e2.tsx', 'const k = <TextInput placeholderTextColor="white" style={{ color: \'black\' }} />;\n'),
  ],
  [
    c('src/app/f.tsx', 'const k = <Text className="text-attention-foreground bg-attention-muted" />;\n'),
    c('src/app/h.tsx', 'export const H = () => <Button size="icon" {...a11y} onPress={go} />;\n'),
    c('src/app/g.tsx', "// Fixes #123 and #abc from the review\nconst k = 'text-muted-foreground';\n"),
    c('src/app/h2.tsx', "const v = 'bg-primary shadow-none';\n"),
    c('src/app/h.tsx', "import { color } from '@/theme/tokens';\nconst tint = color('light', 'foreground');\n"),
    c('src/theme/tokens.ts', "export function color() { return `hsl(${x})`; }\n"),
    c('src/features/capture/source.test.ts', "const meta = { 'theme-color': '#000' };\n"),
  ],
);

// 2+3. No destructive color: errors use attention
draft(
  'errors-use-attention',
  'Show errors and needs-action states with the attention color, not the destructive red; DESIGN.md allows only neutral and attention.',
  { include: SRC, exclude: [...NOT_TESTS, 'src/theme/**', 'src/components/ui/**'] },
  [
    clause('destructive-class', rx(`${L}(?:bg|text|border|ring|fill|stroke|outline|divide)-destructive(?:-foreground)?\\b`), 'DESTRUCTIVE_COLOR',
      'v1 has no red. For an error or needs-action state use text-attention-foreground / bg-attention-muted; for a delete or other destructive action use neutral text-foreground.'),
    clause('destructive-variant', rx(`\\bvariant=\\{?\\s*['"]destructive['"]`), 'DESTRUCTIVE_VARIANT',
      'Use the default or outline Button here; v1 has no red destructive style.'),
  ],
  [
    c('src/app/save.tsx', "{error && <Text className=\"text-destructive text-sm\">{t('save.error')}</Text>}\n"),
    c('src/app/settings.tsx', "<Text className={cn('font-semibold', destructive && 'text-destructive')}>{label}</Text>\n"),
    c('src/app/del.tsx', "<Button variant=\"destructive\" onPress={del}><Text>{t('x')}</Text></Button>\n"),
  ],
  [
    c('src/app/settings.tsx', "Alert.alert(t('x'), '', [{ text: t('settings.deleteConfirm'), style: 'destructive', onPress: runDelete }]);\n"),
    c('src/app/settings2.tsx', 'function Row({ destructive }: { destructive?: boolean }) { return null; }\n'),
    c('src/app/save2.tsx', '{error && <Text className="text-attention-foreground">{msg}</Text>}\n'),
    c('src/theme/tokens.ts', "const t = { destructive: '0 84% 60%' };\n"),
    c('src/components/ui/button.tsx', "const d = 'bg-destructive active:bg-destructive/90';\n"),
  ],
);

// 4+5. Type only through Text variants
const SIZE = `${L}text-(?:(?:xs|sm|base|lg|[2-9]?xl)\\b|\\[[0-9.]+(?:px|rem|em)?\\])`;
const WEIGHT = `${L}font-(?:thin|extralight|light|normal|medium|semibold|bold|extrabold|black)\\b`;
draft(
  'type-through-text-variants',
  'Set text size and weight through a Text variant from the DESIGN.md type scale, not with size or weight classes in screens, features or shared components.',
  { include: SRC, exclude: [...NOT_TESTS, 'src/components/ui/**', 'src/theme/**'] },
  [
    clause('size-class', rx(SIZE), 'TEXT_SIZE_CLASS',
      'Use `<Text variant=...>` for the DESIGN.md style this is (card title, status line, ...); if that style has no variant yet, add it in src/components/ui/text.tsx.'),
    clause('weight-class', rx(WEIGHT), 'FONT_WEIGHT_CLASS',
      'Take the weight from a `<Text variant=...>` on the DESIGN.md type scale; if no variant has it yet, add one in src/components/ui/text.tsx.'),
  ],
  [
    c('src/features/activities/activity-card.tsx', '<Text className="text-[15px] font-bold leading-snug">{item.title}</Text>\n'),
    c('src/app/save.tsx', "{error && <Text className=\"text-destructive text-sm\">{t('save.error')}</Text>}\n"),
    c('src/components/screen-header.tsx', '<Text className="text-[28px] font-extrabold tracking-tight">{title}</Text>\n'),
    c('src/app/tabs.tsx', "<Text className={cn('text-xs', focused ? 'font-bold' : 'font-semibold')}>{label}</Text>\n"),
  ],
  [
    c('src/app/a.tsx', '<Text variant="cardTitle" className="text-muted-foreground text-center">{t}</Text>\n'),
    c('src/app/b.tsx', '<Text className="text-attention-foreground font-sans">{t}</Text>\n'),
    c('src/components/ui/text.tsx', "const v = { cardTitle: 'text-[15px] font-bold' };\n"),
  ],
  'log',
);

// 6. 4-point spacing
const SP = `-?(?:p[xytrblse]?|m[xytrblse]?|gap(?:-[xy])?|space-[xy])`;
draft(
  'spacing-on-4pt-scale',
  'Keep padding, margin and gaps on the 4-point scale (4, 8, 12, 16, 20, 24, 32, 40, 48); no half steps or arbitrary values.',
  { include: SRC, exclude: [...NOT_TESTS, 'src/theme/**'] },
  [
    clause('half-step', rx(`${L}${SP}-[0-9]+\\.5\\b`), 'HALF_STEP_SPACING',
      'Use a whole step instead of this half step: 0.5 becomes 1, 1.5 becomes 1 or 2, 2.5 becomes 2 or 3, 3.5 becomes 3 or 4 (1 step = 4 pt).'),
    clause('arbitrary', rx(`${L}${SP}-\\[[^\\]\\s-][^\\]\\s]*\\]`), 'ARBITRARY_SPACING',
      'Replace this arbitrary value with the nearest step on the 4-point scale (1=4, 2=8, 3=12, 4=16, 5=20, 6=24, 8=32, 10=40, 12=48).'),
    clause('off-scale', rx(`${L}${SP}-(?:7|9|11|1[3-9]|[2-9][0-9]|[1-9][0-9]{2})\\b`), 'OFF_SCALE_SPACING',
      'Use a step on the 4-point scale (1, 2, 3, 4, 5, 6, 8, 10, 12 = 4 to 48 pt). If this clears a tab bar or button, pad by its measured height instead.'),
  ],
  [
    c('src/app/(tabs)/plans.tsx', '<ScrollView contentContainerClassName="gap-3.5 px-5 pb-6 pt-1" />\n'),
    c('src/components/attention-card.tsx', '<Pressable className="flex-row gap-3 px-3.5 py-3" />\n'),
    c('src/app/a.tsx', '<View className="m-[7px]" />\n'),
    c('src/app/b.tsx', '<View className="pt-14 -mt-7" />\n'),
  ],
  [
    c('src/app/c.tsx', '<View className="gap-6 px-5 pb-10 pt-2 p-12 -mt-1 mt-auto space-x-reverse" />\n'),
    c('src/app/d.tsx', '<Button className="h-[52px] size-11 min-w-[88px] top-0.5" />\n'),
    c('src/app/d2.tsx', '<View className="mt-[-1px] border-hairline" />\n'),
  ],
);

// 8. Radius only from the four named radii
draft(
  'radius-from-tokens',
  'Round corners only with the card, sheet, button and chip radii; no Tailwind radius sizes, arbitrary radii or numeric border radius in styles.',
  { include: SRC, exclude: [...NOT_TESTS, 'src/theme/**'] },
  [
    clause('radius-class', rx(`${L}rounded(?:-(?:t|b|l|r|s|e|tl|tr|bl|br))?(?:-(?:sm|md|lg|xl|2xl|3xl|full|\\[[^\\]\\s]+\\]))?${R}`), 'OFF_TOKEN_RADIUS',
      'Use rounded-card (14), rounded-sheet (22), rounded-button (12) or rounded-chip (999), with a side prefix if needed (rounded-t-sheet).'),
    clause('radius-style', rx(`\\bborder(?:Top|Bottom)?(?:Left|Right|Start|End)?Radius\\s*:\\s*[1-9]`), 'NUMERIC_RADIUS_STYLE',
      'Use a rounded-card / rounded-sheet / rounded-button / rounded-chip class, or read the value from src/theme/radius.json, instead of a number.'),
  ],
  [
    c('src/features/activities/activity-card.tsx', '<View className="bg-muted size-12 items-center justify-center rounded-lg" />\n'),
    c('src/components/ui/text.tsx', "const code = 'bg-muted relative rounded px-2 font-mono';\n"),
    c('src/app/a.tsx', '<View className="rounded-t-[22px] rounded-full" />\n'),
    c('src/app/b.tsx', 'const s = StyleSheet.create({ box: { borderTopLeftRadius: 8 } });\n'),
  ],
  [
    c('src/app/c.tsx', '<View className="rounded-card rounded-t-sheet rounded-button rounded-chip rounded-none" />\n'),
    c('src/app/d.tsx', "import radius from '@/theme/radius.json';\nconst s = { borderRadius: radius.card };\n"),
    c('src/app/d0.tsx', 'const s = { borderRadius: 0 };\n'),
  ],
);

// 9. One sheet shadow
draft(
  'no-shadows',
  'Leave shadows off everything but the bottom sheet; cards use a 1 px border instead, as DESIGN.md says.',
  { include: SRC, exclude: [...NOT_TESTS, 'src/theme/**'] },
  [
    clause('shadow-class', rx(`${L}(?:drop-)?shadow(?:-(?:sm|md|lg|xl|2xl|inner|\\[[^\\]\\s]+\\]))?${R}`), 'SHADOW_CLASS',
      'Take this shadow (and its shadow color) off; use a 1 px `border border-border` if it needs an edge. DESIGN.md allows one shadow, on the bottom sheet.'),
    clause('shadow-style', rx(`\\b(?:shadowColor|shadowOpacity|shadowRadius|shadowOffset|elevation|boxShadow)\\s*:`), 'SHADOW_STYLE',
      'Take this shadow style off; use a 1 px `border border-border` for an edge. DESIGN.md allows one shadow, on the bottom sheet; if this is it, keep it.'),
  ],
  [
    c('src/components/ui/button.tsx', "const d = 'bg-primary active:bg-primary/90 shadow-sm shadow-black/5';\n"),
    c('src/app/a.tsx', 'const s = StyleSheet.create({ card: { shadowColor: x, elevation: 2 } });\n'),
    c('src/app/b.tsx', '<View className="shadow rounded-card" />\n'),
    c('src/app/b2.tsx', '<View className="shadow-[0_1px_2px_rgba(0,0,0,0.1)]" />\n'),
  ],
  [
    c('src/components/ui/button2.tsx', "const d = 'rounded-button shadow-none';\n"),
    c('src/app/c.tsx', '<View className="border-border rounded-card border" />\n'),
    c('src/app/c2.tsx', '<View className="drop-shadow-none shadow-none" />\n'),
  ],
);

// 12. Icons only from lucide-react-native
const ICONS = `@expo/vector-icons|react-native-vector-icons|phosphor-react-native|react-native-heroicons|@heroicons/[\\w-]+|@tabler/icons[\\w-]*|react-native-feather|@fortawesome/[\\w-]+|react-icons|@radix-ui/react-icons|iconsax-react-native|expo-symbols|@mdi/[\\w-]+|lucide-react`;
draft(
  'lucide-icons-only',
  'Use icons from lucide-react-native only; no other icon sets in the app or its dependencies.',
  { include: [...SRC, 'package.json'], exclude: NOT_TESTS },
  [
    clause('icon-import', rx(`(?:from\\s+|require\\(\\s*|import\\s+)['"](?:${ICONS})(?:/[^'"]*)?['"]`), 'OTHER_ICON_SET',
      "Import this icon from lucide-react-native and render it with `<Icon as={...} />` from '@/components/ui/icon'."),
    clause('icon-dependency', rx(`"(?:${ICONS})"\\s*:`), 'OTHER_ICON_DEPENDENCY',
      'Drop this icon package and use lucide-react-native, the only icon set DESIGN.md allows.'),
  ],
  [
    c('src/app/a.tsx', "import { Ionicons } from '@expo/vector-icons';\n"),
    c('src/app/b.tsx', "import { House } from 'lucide-react';\n"),
    c('src/app/b3.tsx', "import { SymbolView } from 'expo-symbols';\n"),
    c('package.json', '{\n  "dependencies": {\n    "phosphor-react-native": "^2.0.0"\n  }\n}\n'),
  ],
  [
    c('src/app/c.tsx', "import { ChevronLeft, Plus } from 'lucide-react-native';\n"),
    c('package.json', '{\n  "dependencies": {\n    "lucide-react-native": "^1.62.0"\n  }\n}\n'),
    c('src/app/d.tsx', "// we chose lucide-react-native over @expo/vector-icons\nconst x = 1;\n"),
  ],
);

// 15. Icon-only buttons need a label
const LABEL = { kind: 'jsx_attribute', regex: '^(?:accessibilityLabel|aria-label)\\b' };
draft(
  'icon-buttons-have-labels',
  'Give every icon-only button a screen reader label, both icon-size Buttons and Pressables that hold only an icon.',
  { include: ['src/**/*.tsx'], exclude: [...NOT_TESTS, 'src/components/ui/**'] },
  [
    clause('icon-button', { ast: { language: 'typescript', rule: {
      any: [{ kind: 'jsx_opening_element' }, { kind: 'jsx_self_closing_element' }],
      regex: '^<Button\\b',
      has: { kind: 'jsx_attribute', regex: '^size=\\{?["\']icon' },
      not: { has: { any: [LABEL, { kind: 'jsx_expression', regex: '^\\{\\s*\\.\\.\\.' }] } },
    } } }, 'ICON_BUTTON_NO_LABEL',
      "Add accessibilityLabel={t('...')} to this icon-only Button, with the string in locales/en.json and fr.json."),
    clause('icon-pressable', { ast: { language: 'typescript', rule: {
      kind: 'jsx_element',
      has: { kind: 'jsx_opening_element', regex: '^<Pressable\\b', not: { has: { any: [LABEL, { kind: 'jsx_expression', regex: '^\\{\\s*\\.\\.\\.' }] } } },
      all: [
        { has: { kind: 'jsx_self_closing_element', regex: '^<Icon\\b' } },
        { not: { has: { kind: 'jsx_element' } } },
        { not: { has: { kind: 'jsx_expression' } } },
        { not: { has: { kind: 'jsx_text', regex: '\\S' } } },
      ],
    } } }, 'ICON_PRESSABLE_NO_LABEL',
      "Add accessibilityLabel={t('...')} to this Pressable; it holds only an icon, so a screen reader has nothing to read."),
  ],
  [
    c('src/app/a.tsx', 'export const A = () => (\n  <Button variant="ghost" size="icon-sheet" onPress={go}>\n    <Icon as={ChevronLeft} size={22} />\n  </Button>\n);\n'),
    c('src/app/b.tsx', 'export const B = () => (\n  <Pressable onPress={back} className="size-11 items-center justify-center">\n    <Icon as={ChevronLeft} size={22} />\n  </Pressable>\n);\n'),
    c('src/app/c.tsx', 'export const C = () => <Button size="icon" onPress={go} />;\n'),
  ],
  [
    c('src/app/d.tsx', "export const D = () => (\n  <Button variant=\"ghost\" size=\"icon-sheet\" accessibilityLabel={t('child.nextMonth')} onPress={go}>\n    <Icon as={ChevronRight} size={22} />\n  </Button>\n);\n"),
    c('src/app/e.tsx', 'export const E = () => (\n  <Pressable onPress={go} className="h-12">\n    <Icon as={icon} size={22} />\n    <Text>{label}</Text>\n  </Pressable>\n);\n'),
    c('src/app/f.tsx', "export const F = () => (\n  <Pressable accessibilityLabel={t('settings.back')} onPress={back}>\n    <Icon as={ChevronLeft} size={22} />\n  </Pressable>\n);\n"),
    c('src/app/g.tsx', 'export const G = () => <Button size="sheet" onPress={go}><Text>{t(\'x\')}</Text></Button>;\n'),
  ],
);
