from qwertone_analyzer import theory


def test_note_names_match_rust_app():
    assert theory.note_name(60) == "C4"
    assert theory.note_name(61) == "C#4"
    assert theory.note_name(48) == "C3"
    assert theory.note_name(21) == "A0"


def test_chord_symbols():
    assert theory.chord_symbol(9, "min") == "Am"
    assert theory.chord_symbol(0, "maj", 4) == "C/E"
    assert theory.chord_symbol(7, "7") == "G7"
    assert theory.chord_symbol(None, None) == "N"


def test_roman_numerals_major_and_minor():
    g = 7
    assert theory.roman_numeral(7, "maj", g, "major") == "I"
    assert theory.roman_numeral(4, "min", g, "major") == "vi"
    assert theory.roman_numeral(6, "dim", g, "major") == "vii°"
    assert theory.roman_numeral(5, "maj", g, "major") == "bVII"  # F in G major (borrowed)
    a = 9
    assert theory.roman_numeral(0, "maj", a, "minor") == "III"
    assert theory.roman_numeral(4, "7", a, "minor") == "V7"
    assert theory.roman_numeral(2, "min", a, "minor") == "iv"


def test_key_estimate_from_scale_histogram():
    hist = [0.0] * 12
    for pc in (7, 9, 11, 0, 2, 4, 6):  # G major scale
        hist[pc] = 1.0
    for pc in (7, 11, 2):  # tonic triad heavier
        hist[pc] += 1.0
    r, tonic, mode = theory.estimate_key(hist)[0]
    assert (tonic, mode) == (7, "major")
