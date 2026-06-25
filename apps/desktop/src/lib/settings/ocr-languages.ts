// Shared OCR-language list, sourced once so the Settings panel and the
// onboarding flow present the same searchable options (Settings used a
// free-text input, onboarding a short curated Select — both now use a Combobox
// over this list, mirroring transcription-languages.ts). These are the common
// Tesseract `traineddata` codes, labelled "Name (code)" so a search matches
// either the English name or the Tesseract code.
export interface OcrLanguageOption {
  value: string;
  label: string;
}

export const OCR_LANGUAGE_OPTIONS: OcrLanguageOption[] = [
  { value: "eng", label: "English (eng)" },
  { value: "fra", label: "French (fra)" },
  { value: "deu", label: "German (deu)" },
  { value: "spa", label: "Spanish (spa)" },
  { value: "ita", label: "Italian (ita)" },
  { value: "por", label: "Portuguese (por)" },
  { value: "nld", label: "Dutch (nld)" },
  { value: "jpn", label: "Japanese (jpn)" },
  { value: "chi_sim", label: "Chinese, Simplified (chi_sim)" },
];
