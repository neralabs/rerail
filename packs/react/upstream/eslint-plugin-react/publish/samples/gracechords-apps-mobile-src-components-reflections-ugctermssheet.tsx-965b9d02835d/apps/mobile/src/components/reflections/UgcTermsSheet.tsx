import { useState } from 'react'
import { ActivityIndicator, Alert, Linking, Pressable, ScrollView, Text, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'
import { useTranslation } from 'react-i18next'
import FormSheetShell from '../FormSheetShell'
import SymbolIcon from '../SymbolIcon'
import { useFormSheet } from '../../lib/formSheetHost'
import { useTheme } from '../../theme/ThemeProvider'
import { errMessage } from '../../lib/errors'
import { acceptUgcTerms } from '../../lib/ugc'
import { recordAgeRange, useAgeGate, type AgeRange } from '../../lib/ageGate'
import { supabase } from '../../lib/supabase'
import {
  TERMS_URL,
  UGC_GATE_CONFIRM_PREFIX,
  UGC_GATE_ENFORCEMENT,
  UGC_GATE_INTRO,
  UGC_GATE_RULES,
  UGC_GATE_SUBHEAD,
} from '../../lib/ugcTerms'

// The Apple 1.2 UGC acceptance gate, extended to also capture age assurance for
// the public Shared Reflections feed. It serves as the one-time unlock gate for
// the whole public section (view + post):
//   • When the user's age isn't known yet, it asks a neutral Under-13 / 13+
//     question. "Under 13" records the range and closes — public stays hidden.
//   • When age is already known (a prior attestation, or a Declared Age Range API
//     seed passed via `seededAgeRange`), it shows the terms only.
// "Agree & Share" records 13+ (if needed) + terms acceptance, then calls
// onAgreed() to proceed (post, or simply reveal the feed at the landing gate).
type UgcProps = {
  visible: boolean
  onClose: () => void
  /** Called after 13+ age + terms are recorded — proceed (post / reveal feed). */
  onAgreed: () => void
  /**
   * Age already determined by Apple's Declared Age Range API. When set, the sheet
   * skips the age question and records this range (source 'declared_api') on agree.
   */
  seededAgeRange?: AgeRange
  /** Called after "Under 13" is recorded (public stays hidden). */
  onDeclined?: () => void
}

export default function UgcTermsSheet(props: UgcProps) {
  useFormSheet(props.visible, () => <UgcContent {...props} />, props.onClose)
  return null
}

function AgeOption({
  label,
  selected,
  onPress,
}: {
  label: string
  selected: boolean
  onPress: () => void
}) {
  const t = useTheme()
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ selected }}
      style={({ pressed }) => ({
        flexDirection: 'row',
        alignItems: 'center',
        gap: t.spacing.md,
        paddingVertical: t.spacing.md,
        paddingHorizontal: t.spacing.lg,
        borderRadius: t.radii.md,
        borderWidth: 1,
        borderColor: selected ? t.colors.accent : t.colors.border,
        backgroundColor: selected ? t.colors.accentSoft : t.colors.surface,
        opacity: pressed ? 0.85 : 1,
      })}
    >
      <View
        style={{
          width: 22,
          height: 22,
          borderRadius: 11,
          borderWidth: 2,
          borderColor: selected ? t.colors.accent : t.colors.border,
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        {selected ? (
          <View
            style={{ width: 12, height: 12, borderRadius: 6, backgroundColor: t.colors.accent }}
          />
        ) : null}
      </View>
      <Text style={{ flex: 1, fontSize: 15, fontWeight: '600', color: t.colors.ink }}>{label}</Text>
    </Pressable>
  )
}

function UgcContent({ onClose, onAgreed, onDeclined, seededAgeRange }: UgcProps) {
  const t = useTheme()
  const { t: tx } = useTranslation('reader')
  const insets = useSafeAreaInsets()
  const age = useAgeGate()
  const [busy, setBusy] = useState(false)
  const [picked, setPicked] = useState<AgeRange | null>(null)

  // Ask for age only when it isn't already known (no seed, and none stored). At
  // the compose gate the user is already 13+, so this shows terms only.
  const askAge = seededAgeRange == null && age.ready && age.range == null
  const effectiveRange: AgeRange | null = seededAgeRange ?? (askAge ? picked : age.range ?? '13_plus')
  const isUnder13 = effectiveRange === 'under_13'
  const canConfirm = !busy && (!askAge || picked != null)

  const record = async () => {
    if (!canConfirm) return
    setBusy(true)
    try {
      if (isUnder13) {
        await recordAgeRange(supabase, 'under_13', 'self')
        onDeclined?.()
        onClose()
        return
      }
      // 13+ path: record the range (unless already stored) then accept terms.
      if (age.range !== '13_plus') {
        await recordAgeRange(supabase, '13_plus', seededAgeRange ? 'declared_api' : 'self')
      }
      await acceptUgcTerms(supabase)
      onAgreed()
    } catch (err: unknown) {
      setBusy(false)
      Alert.alert(tx('ugc.errorTitle'), errMessage(err))
    }
  }

  const confirmLabel = isUnder13 ? tx('ugc.ageUnder13Confirm') : tx('ugc.agree')

  return (
    <FormSheetShell title={tx('ugc.title')} actionLabel={tx('ugc.cancel')} onAction={onClose}>
      <ScrollView
        contentContainerStyle={{
          paddingHorizontal: t.spacing.lg,
          paddingTop: t.spacing.lg,
          // Clear the home indicator so the sheet fills to the screen bottom
          // instead of leaving a gap below the button.
          paddingBottom: insets.bottom + t.spacing.lg,
          gap: t.spacing.md,
        }}
        style={{ maxHeight: 480 }}
        contentContainerStyle={{ padding: t.spacing.lg, gap: t.spacing.md }}
      >
        {askAge ? (
          <View style={{ gap: t.spacing.sm }}>
            <Text style={{ fontSize: 15, fontWeight: '600', color: t.colors.ink }}>
              {tx('ugc.ageHeading')}
            </Text>
            <AgeOption
              label={tx('ugc.age13Plus')}
              selected={picked === '13_plus'}
              onPress={() => setPicked('13_plus')}
            />
            <AgeOption
              label={tx('ugc.ageUnder13')}
              selected={picked === 'under_13'}
              onPress={() => setPicked('under_13')}
            />
            {isUnder13 ? (
              <Text style={{ fontSize: 13.5, lineHeight: 20, color: t.colors.muted }}>
                {tx('ugc.ageUnder13Notice')}
              </Text>
            ) : null}
          </View>
        ) : null}

        {/* Terms are hidden once the user has said they're under 13 — there's
            nothing to agree to. */}
        {!isUnder13 ? (
          <>
            <Text style={{ fontSize: 15, lineHeight: 22, color: t.colors.sec }}>{UGC_GATE_INTRO}</Text>
            <Text style={{ fontSize: 15, fontWeight: '600', color: t.colors.ink }}>{UGC_GATE_SUBHEAD}</Text>
            <View style={{ gap: 8 }}>
              {UGC_GATE_RULES.map((rule, i) => (
                <View key={i} style={{ flexDirection: 'row', gap: 8 }}>
                  <SymbolIcon name="checkmark.circle.fill" size={16} color={t.colors.accent} />
                  <Text style={{ flex: 1, fontSize: 14.5, lineHeight: 21, color: t.colors.ink }}>{rule}</Text>
                </View>
              ))}
            </View>
            <Text style={{ fontSize: 13.5, lineHeight: 20, color: t.colors.muted }}>{UGC_GATE_ENFORCEMENT}</Text>
            <Text style={{ fontSize: 13.5, lineHeight: 20, color: t.colors.muted }}>
              {UGC_GATE_CONFIRM_PREFIX}
              <Text
                style={{ color: t.colors.accent, fontWeight: '600' }}
                onPress={() => void Linking.openURL(TERMS_URL)}
              >
                {tx('ugc.termsLink')}
              </Text>
              .
            </Text>
          </>
        ) : null}

        <Pressable
          onPress={record}
          disabled={!canConfirm}
          accessibilityRole="button"
          style={{
            height: 50,
            borderRadius: t.radii.md,
            backgroundColor: t.colors.accent,
            alignItems: 'center',
            justifyContent: 'center',
            marginTop: t.spacing.sm,
            opacity: canConfirm ? 1 : 0.6,
          }}
        >
          {busy ? (
            <ActivityIndicator color={t.colors.onAccent} />
          ) : (
            <Text style={{ fontSize: 16, fontWeight: '700', color: t.colors.onAccent }}>
              {confirmLabel}
            </Text>
          )}
        </Pressable>
      </ScrollView>
    </FormSheetShell>
  )
}
