# Suggested selections of resource packs (ADR 0013), offered with `s` on
# the Resource packs screen: one key ticks every pack of a set that is not
# installed yet. Sizes are shown in the interface, computed from the catalog.
# Pack ids are checked at build time.
#
# The roadmap's "Geography pack" waits for maps (Phase 5); Wikivoyage is in
# the Essential sets meanwhile.
catalog:
let
  inCategories =
    cats: lang:
    map (p: p.id) (builtins.filter (p: builtins.elem p.category cats && p.language == lang) catalog);
in
[
  {
    id = "essential-en";
    title = "Essential (English)";
    description = "A good start for most people: the most-read Wikipedia articles, medicine, medicines, travel and practical guides, emergency preparedness.";
    packs = [
      "wikipedia-en-top"
      "wikipedia-en-medicine"
      "nhs-medicines"
      "wikivoyage-en"
      "appropedia"
      "ready-gov"
    ];
  }
  {
    id = "essentiel-fr";
    title = "Essentiel (français)";
    description = "Pour commencer : les articles de Wikipédia les plus lus, la médecine, le dictionnaire et le guide de voyage.";
    packs = [
      "wikipedia-fr-top"
      "wikipedia-fr-medicine"
      "wiktionary-fr"
      "wikivoyage-fr"
    ];
  }
  {
    id = "field-guides";
    title = "Field guides (English)";
    description = "Every health and practical pack: medicine, water, food, energy, repairs, vehicles, weather, radio, emergencies.";
    packs = inCategories [ "health" "practical" ] "en";
  }
  {
    id = "knowledge-en";
    title = "Knowledge (English)";
    description = "The complete English Wikipedia (text only) and Wiktionary. Very large: about 63 GiB.";
    packs = [
      "wikipedia-en-all"
      "wiktionary-en"
    ];
  }
  {
    id = "connaissances-fr";
    title = "Connaissances (français)";
    description = "Toute la Wikipédia en français (texte seul) et le Wiktionnaire. Environ 17 GiB.";
    packs = [
      "wikipedia-fr-all"
      "wiktionary-fr"
    ];
  }
  {
    id = "documentation";
    title = "Extended documentation";
    description = "Computer documentation: Linux manual pages, Bash, Nix, ArchWiki and the Unix and Super User question sites.";
    packs = map (p: p.id) (
      builtins.filter (
        p:
        builtins.elem p.category [
          "computing"
          "documentation"
        ]
      ) catalog
    );
  }
  {
    id = "complete";
    title = "Complete";
    description = "Every pack in the catalog. Needs well over 100 GiB.";
    packs = map (p: p.id) catalog;
  }
]
