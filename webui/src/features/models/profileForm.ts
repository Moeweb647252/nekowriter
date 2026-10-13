import type { ModelProfile } from "@/api/generated/ModelProfile"

/** A model profile being edited; numbers stay text while the user types them. */
export interface ProfileForm {
  modelId: string
  contextWindow: string
  maxOutputTokens: string
  supportsThinking: boolean
  reportsStreamUsage: boolean
}

export function profileForm(profile: ModelProfile | null): ProfileForm {
  return {
    modelId: profile?.modelId ?? "",
    contextWindow: String(profile?.contextWindow ?? 128_000),
    maxOutputTokens: String(profile?.maxOutputTokens ?? 8_192),
    supportsThinking: profile?.supportsThinking ?? false,
    reportsStreamUsage: profile?.reportsStreamUsage ?? true,
  }
}

/** The profile to send. The server validates it and names the field of any problem. */
export function profileFromForm(form: ProfileForm): ModelProfile {
  const count = (text: string) => {
    const value = Math.trunc(Number(text))
    return Number.isFinite(value) && value > 0 ? value : 0
  }
  return {
    modelId: form.modelId,
    contextWindow: count(form.contextWindow),
    maxOutputTokens: count(form.maxOutputTokens),
    supportsThinking: form.supportsThinking,
    reportsStreamUsage: form.reportsStreamUsage,
  }
}
