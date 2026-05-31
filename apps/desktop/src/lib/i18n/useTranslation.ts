// React binding for the i18n runtime. Components call `useTranslation()` and use
// the returned `t(key, params?)` to render localized strings. The function is
// reactive: switching `locale` in the store re-renders every consumer.

import { useCallback } from "react";
import { useLuxStore } from "../store";
import { translate, type Locale, type MessageKey, type MessageParams } from "./index";

export type TranslateFn = (key: MessageKey, params?: MessageParams) => string;

export function useTranslation(): { t: TranslateFn; locale: Locale } {
  const locale = useLuxStore((state) => state.locale);
  const t = useCallback<TranslateFn>((key, params) => translate(locale, key, params), [locale]);
  return { t, locale };
}
