// Whisper-supported languages.
//
// Source: the LANGUAGES dict in OpenAI's whisper/tokenizer.py (99 entries),
// plus Cantonese ("yue") added in large-v3 / large-v3-turbo. Same coverage
// for the cloud API and our bundled on-device model — whisper.cpp passes
// the code straight to the multilingual model, no separate allowlist.
//
// `value` is the ISO code Whisper expects (a few non-639-1 exceptions:
// haw, jw, yue). `label` is the English name; `native` is the native form
// for languages where it differs from the English name and is widely
// recognizable. The picker shows "label (native)" when native is set.
//
// "auto" is a frontend sentinel — both providers detect it and let
// Whisper auto-detect the language.

export type Language = {
  value: string;
  label: string;
  native?: string;
};

// Auto first, then alphabetical by English name.
export const LANGUAGES: Language[] = [
  { value: "auto", label: "Auto-detect" },
  { value: "af", label: "Afrikaans" },
  { value: "sq", label: "Albanian", native: "Shqip" },
  { value: "am", label: "Amharic", native: "አማርኛ" },
  { value: "ar", label: "Arabic", native: "العربية" },
  { value: "hy", label: "Armenian", native: "Հայերեն" },
  { value: "as", label: "Assamese", native: "অসমীয়া" },
  { value: "az", label: "Azerbaijani", native: "Azərbaycan" },
  { value: "ba", label: "Bashkir", native: "Башҡортса" },
  { value: "eu", label: "Basque", native: "Euskara" },
  { value: "be", label: "Belarusian", native: "Беларуская" },
  { value: "bn", label: "Bengali", native: "বাংলা" },
  { value: "bs", label: "Bosnian", native: "Bosanski" },
  { value: "br", label: "Breton", native: "Brezhoneg" },
  { value: "bg", label: "Bulgarian", native: "Български" },
  { value: "my", label: "Burmese", native: "မြန်မာ" },
  { value: "yue", label: "Cantonese", native: "粵語" },
  { value: "ca", label: "Catalan", native: "Català" },
  { value: "zh", label: "Chinese", native: "中文" },
  { value: "hr", label: "Croatian", native: "Hrvatski" },
  { value: "cs", label: "Czech", native: "Čeština" },
  { value: "da", label: "Danish", native: "Dansk" },
  { value: "nl", label: "Dutch", native: "Nederlands" },
  { value: "en", label: "English" },
  { value: "et", label: "Estonian", native: "Eesti" },
  { value: "fo", label: "Faroese", native: "Føroyskt" },
  { value: "fi", label: "Finnish", native: "Suomi" },
  { value: "fr", label: "French", native: "Français" },
  { value: "gl", label: "Galician", native: "Galego" },
  { value: "ka", label: "Georgian", native: "ქართული" },
  { value: "de", label: "German", native: "Deutsch" },
  { value: "el", label: "Greek", native: "Ελληνικά" },
  { value: "gu", label: "Gujarati", native: "ગુજરાતી" },
  { value: "ht", label: "Haitian Creole", native: "Kreyòl Ayisyen" },
  { value: "ha", label: "Hausa" },
  { value: "haw", label: "Hawaiian", native: "ʻŌlelo Hawaiʻi" },
  { value: "he", label: "Hebrew", native: "עברית" },
  { value: "hi", label: "Hindi", native: "हिन्दी" },
  { value: "hu", label: "Hungarian", native: "Magyar" },
  { value: "is", label: "Icelandic", native: "Íslenska" },
  { value: "id", label: "Indonesian", native: "Bahasa Indonesia" },
  { value: "it", label: "Italian", native: "Italiano" },
  { value: "ja", label: "Japanese", native: "日本語" },
  { value: "jw", label: "Javanese", native: "Basa Jawa" },
  { value: "kn", label: "Kannada", native: "ಕನ್ನಡ" },
  { value: "kk", label: "Kazakh", native: "Қазақша" },
  { value: "km", label: "Khmer", native: "ខ្មែរ" },
  { value: "ko", label: "Korean", native: "한국어" },
  { value: "lo", label: "Lao", native: "ລາວ" },
  { value: "la", label: "Latin", native: "Latina" },
  { value: "lv", label: "Latvian", native: "Latviešu" },
  { value: "ln", label: "Lingala" },
  { value: "lt", label: "Lithuanian", native: "Lietuvių" },
  { value: "lb", label: "Luxembourgish", native: "Lëtzebuergesch" },
  { value: "mk", label: "Macedonian", native: "Македонски" },
  { value: "mg", label: "Malagasy" },
  { value: "ms", label: "Malay", native: "Bahasa Melayu" },
  { value: "ml", label: "Malayalam", native: "മലയാളം" },
  { value: "mt", label: "Maltese", native: "Malti" },
  { value: "mi", label: "Maori", native: "Te Reo Māori" },
  { value: "mr", label: "Marathi", native: "मराठी" },
  { value: "mn", label: "Mongolian", native: "Монгол" },
  { value: "ne", label: "Nepali", native: "नेपाली" },
  { value: "no", label: "Norwegian", native: "Norsk" },
  { value: "nn", label: "Nynorsk" },
  { value: "oc", label: "Occitan", native: "Occitan" },
  { value: "ps", label: "Pashto", native: "پښتو" },
  { value: "fa", label: "Persian", native: "فارسی" },
  { value: "pl", label: "Polish", native: "Polski" },
  { value: "pt", label: "Portuguese", native: "Português" },
  { value: "pa", label: "Punjabi", native: "ਪੰਜਾਬੀ" },
  { value: "ro", label: "Romanian", native: "Română" },
  { value: "ru", label: "Russian", native: "Русский" },
  { value: "sa", label: "Sanskrit", native: "संस्कृतम्" },
  { value: "sr", label: "Serbian", native: "Српски" },
  { value: "sn", label: "Shona", native: "ChiShona" },
  { value: "sd", label: "Sindhi", native: "سنڌي" },
  { value: "si", label: "Sinhala", native: "සිංහල" },
  { value: "sk", label: "Slovak", native: "Slovenčina" },
  { value: "sl", label: "Slovenian", native: "Slovenščina" },
  { value: "so", label: "Somali", native: "Soomaali" },
  { value: "es", label: "Spanish", native: "Español" },
  { value: "su", label: "Sundanese", native: "Basa Sunda" },
  { value: "sw", label: "Swahili", native: "Kiswahili" },
  { value: "sv", label: "Swedish", native: "Svenska" },
  { value: "tl", label: "Tagalog" },
  { value: "tg", label: "Tajik", native: "Тоҷикӣ" },
  { value: "ta", label: "Tamil", native: "தமிழ்" },
  { value: "tt", label: "Tatar", native: "Татарча" },
  { value: "te", label: "Telugu", native: "తెలుగు" },
  { value: "th", label: "Thai", native: "ไทย" },
  { value: "bo", label: "Tibetan", native: "བོད་སྐད་" },
  { value: "tr", label: "Turkish", native: "Türkçe" },
  { value: "tk", label: "Turkmen", native: "Türkmençe" },
  { value: "uk", label: "Ukrainian", native: "Українська" },
  { value: "ur", label: "Urdu", native: "اردو" },
  { value: "uz", label: "Uzbek", native: "Oʻzbekcha" },
  { value: "vi", label: "Vietnamese", native: "Tiếng Việt" },
  { value: "cy", label: "Welsh", native: "Cymraeg" },
  { value: "yi", label: "Yiddish", native: "ייִדיש" },
  { value: "yo", label: "Yoruba", native: "Yorùbá" },
];

export function languageOptionLabel(lang: Language): string {
  if (!lang.native || lang.native === lang.label) return lang.label;
  return `${lang.label} (${lang.native})`;
}
