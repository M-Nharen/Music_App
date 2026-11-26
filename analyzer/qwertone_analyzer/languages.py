"""Which language a lyric line is in, and the codes other tools need.

Languages are BCP 47 tags: `ta`, `te`, `hi`, `en` ..., and `<lang>-Latn` for
an Indian language written in Latin letters (`ta-Latn` = Tanglish, `hi-Latn`
= Hinglish, `te-Latn` = Tenglish). `und` = unknown.

A line in a native script is recognised by its script (Tamil, Telugu,
Devanagari ...). Latin text is English unless it has enough common words
of a romanised Indian language. Whisper's language id (asr.py) decides the
song language when it ran; lines still get their own language from their
text, so a Hindi song with English lines has both.
"""

from __future__ import annotations

import re

# Unicode block of each Indic script -> (aksharamukha script name, default language)
SCRIPTS = {
    "Tamil": ("஀", "௿", "ta"),
    "Telugu": ("ఀ", "౿", "te"),
    "Devanagari": ("ऀ", "ॿ", "hi"),
    "Kannada": ("ಀ", "೿", "kn"),
    "Malayalam": ("ഀ", "ൿ", "ml"),
    "Bengali": ("ঀ", "৿", "bn"),
    "Gujarati": ("઀", "૿", "gu"),
    "Gurmukhi": ("਀", "੿", "pa"),
    "Oriya": ("଀", "୿", "or"),
}
# Languages written in each script (to pick the script for a language code)
LANG_SCRIPT = {"ta": "Tamil", "te": "Telugu", "hi": "Devanagari", "mr": "Devanagari", "ne": "Devanagari",
               "sa": "Devanagari", "kn": "Kannada", "ml": "Malayalam", "bn": "Bengali", "as": "Bengali",
               "gu": "Gujarati", "pa": "Gurmukhi", "or": "Oriya"}
OTHER_SCRIPTS = [  # non-Indic scripts Whisper commonly writes; enough to tell languages apart
    ("ar", "؀", "ۿ"), ("ru", "Ѐ", "ӿ"), ("el", "Ͱ", "Ͽ"), ("he", "֐", "׿"), ("th", "฀", "๿"),
    ("ko", "가", "힯"), ("ja", "぀", "ヿ"), ("zh", "一", "鿿"), ("si", "඀", "෿"),
]

# Common words of romanised Indian languages (to tell Tanglish / Hinglish / Tenglish from English)
LATIN_WORDS = {
    "ta-Latn": set("""
naan nee neeyum ennai unnai enna enakku unakku ennoda unnoda enge inge ange ippo eppo eppadi yaar
illa illai irukku iruku irundhu vaa vaadi vaada po podi poda di da thaan dhaan kooda mattum romba
kaadhal kadhal kaathal anbe anbu uyire uyir nenjam nenje kannu kanne kannil kangal penne ponnu paiyan
vazhkai vaazhkai pesu pesum paadu paattu paatu solla sollu sonna vandha vandhen pogum poren varen
manasula manam nila vaanam mazhai kaatru kaathu thendral malar poo pookkal kanavu kanavil
azhagu azhagaa azhage ennamo yedho edho ellam ellaam onnu rendu konjam nalla ketta thirumba
""".split()),
    "hi-Latn": set("""
hai hain main mera meri mere tera teri tere tum tumhe tumhara tujhe mujhe tujh mujh dil pyaar pyar ishq
kya kyun kyon nahi nahin hum ho mein jaana jaan yaar zindagi saath raat aaj kabhi sanam aankhon
aankhein dard rab chal bas sapna sapne dekho dekha kaise kahan yahan wahan jab tab phir abhi koi
mohabbat duniya khuda rehna kehna suno sun aaja dilbar humko tumko apna apni hoon tha thi
""".split()),
    "te-Latn": set("""
nenu nuvvu naaku neeku prema ante emiti enti ledu undi vacchi kaadu cheppu chesi gundello ninnu nannu
oka kosam chusi choodu andam priya cheli raa vellu ippudu eppudu ela ekkada nee naa manasulo
yedalo kalalo navvu ammayi abbayi bangaram chinni chilipi oohalu jagame ee aa
""".split()),
}

WHISPER_TO_FAMILY = {"ur": "hi"}  # Whisper often calls sung Hindi "Urdu"; see asr.whisper_language

_WORD = re.compile(r"[a-z]+")


def script_of(ch: str) -> str | None:
    """Aksharamukha script name of an Indic character, else None."""
    for name, (lo, hi, _) in SCRIPTS.items():
        if lo <= ch <= hi:
            return name
    return None


def base(lang: str | None) -> str | None:
    """`ta-Latn` -> `ta`; `en` -> `en`."""
    return lang.split("-")[0] if lang else lang


def is_latin_form(lang: str | None) -> bool:
    return bool(lang) and lang.endswith("-Latn")


def has_indic(text: str) -> bool:
    return any(script_of(c) for c in text)


def _latin_language(text: str, song_hint: str | None) -> str:
    if is_latin_form(song_hint):
        return song_hint
    words = _WORD.findall(text.lower())
    if not words:
        return "en"
    best, best_hits = "en", 0
    for lang, vocab in LATIN_WORDS.items():
        hits = sum(1 for w in words if w in vocab)
        if hits > best_hits:
            best, best_hits = lang, hits
    if best != "en" and (best_hits >= 2 or best_hits / len(words) >= 0.25):
        # a song known to be Tamil/Telugu/Hindi decides between overlapping word lists
        if song_hint and base(song_hint) in ("ta", "te", "hi") and best_hits < 3:
            return f"{base(song_hint)}-Latn"
        return best
    return "en"


def line_language(text: str, song_hint: str | None = None) -> str:
    """Language of one lyric line. `song_hint` breaks ties (song language, or `--language`)."""
    letters = [c for c in text if c.isalpha() or "ऀ" <= c <= "෿"]
    if not letters:
        return song_hint or "und"
    counts: dict[str, int] = {}
    for c in letters:
        s = script_of(c)
        if s:
            counts[s] = counts.get(s, 0) + 1
    if counts:
        script = max(counts, key=counts.get)
        if counts[script] / len(letters) >= 0.3:
            default = SCRIPTS[script][2]
            # Devanagari is also Marathi/Nepali, Bengali also Assamese: trust the song language then
            if song_hint and LANG_SCRIPT.get(base(song_hint)) == script:
                return base(song_hint)
            return default
    for lang, lo, hi in OTHER_SCRIPTS:
        if sum(1 for c in letters if lo <= c <= hi) / len(letters) >= 0.3:
            if lang == "ar" and song_hint in ("ur", "fa"):
                return song_hint
            return lang
    latin = sum(1 for c in letters if c.isascii())
    if latin / len(letters) >= 0.7:
        return _latin_language(text, song_hint)
    return song_hint or "und"


def song_languages(lines: list[dict]) -> list[str]:
    """Languages of the lines, most lines first (und left out)."""
    counts: dict[str, int] = {}
    for l in lines:
        if l.get("language") and l["language"] != "und":
            counts[l["language"]] = counts.get(l["language"], 0) + 1
    return sorted(counts, key=lambda k: -counts[k])


def song_language(lines: list[dict], override: str | None = None, detected: str | None = None,
                  tag: str | None = None) -> str:
    """One language for the song: --language, else the lines (a song with a few English
    lines is still a Tamil/Hindi/... song), else Whisper's guess, else the LRC [la:] tag."""
    if override:
        return override
    counts: dict[str, int] = {}
    for l in lines:
        counts[l["language"]] = counts.get(l["language"], 0) + 1
    counts.pop("und", None)
    if counts:
        total = sum(counts.values())
        # group native + Latin forms of one language (ta + ta-Latn)
        fam: dict[str, int] = {}
        for k, v in counts.items():
            fam[base(k)] = fam.get(base(k), 0) + v
        non_en = {k: v for k, v in fam.items() if k != "en"}
        if non_en:
            top = max(non_en, key=non_en.get)
            if non_en[top] >= 0.3 * total:
                native, latin = counts.get(top, 0), counts.get(f"{top}-Latn", 0)
                return top if native >= latin else f"{top}-Latn"
        return max(counts, key=counts.get)
    return detected or tag or "und"


# NLLB-200 (FLORES-200) codes, for translation. Covers every language Whisper
# knows that NLLB has too; romanised forms (ta-Latn ...) have none.
NLLB = {
    "ta": "tam_Taml", "te": "tel_Telu", "hi": "hin_Deva", "kn": "kan_Knda", "ml": "mal_Mlym",
    "bn": "ben_Beng", "gu": "guj_Gujr", "pa": "pan_Guru", "mr": "mar_Deva", "or": "ory_Orya",
    "as": "asm_Beng", "ne": "npi_Deva", "sa": "san_Deva", "si": "sin_Sinh", "ur": "urd_Arab",
    "en": "eng_Latn", "es": "spa_Latn", "fr": "fra_Latn", "de": "deu_Latn", "it": "ita_Latn",
    "pt": "por_Latn", "nl": "nld_Latn", "ru": "rus_Cyrl", "uk": "ukr_Cyrl", "pl": "pol_Latn",
    "cs": "ces_Latn", "sk": "slk_Latn", "ro": "ron_Latn", "hu": "hun_Latn", "el": "ell_Grek",
    "tr": "tur_Latn", "ar": "arb_Arab", "fa": "pes_Arab", "he": "heb_Hebr", "ja": "jpn_Jpan",
    "ko": "kor_Hang", "zh": "zho_Hans", "th": "tha_Thai", "vi": "vie_Latn", "id": "ind_Latn",
    "ms": "zsm_Latn", "tl": "tgl_Latn", "sw": "swh_Latn", "sv": "swe_Latn", "no": "nob_Latn",
    "da": "dan_Latn", "fi": "fin_Latn", "bg": "bul_Cyrl", "sr": "srp_Cyrl", "hr": "hrv_Latn",
    "ca": "cat_Latn", "my": "mya_Mymr", "km": "khm_Khmr", "lo": "lao_Laoo", "am": "amh_Ethi",
}


def nllb_code(lang: str | None) -> str | None:
    if not lang or is_latin_form(lang):
        return None
    return NLLB.get(base(lang))
