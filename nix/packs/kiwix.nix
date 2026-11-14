# The resource packs offered by default (ADR 0013): Kiwix ZIM files.
#
# Each entry pins the exact size and SHA-256 published by Kiwix in the
# file's metalink (`<url>.meta4`, retrieved 2026-11-14). The store installs a
# file only if its bytes match. Kiwix removes old files from its servers
# after a few months: a pack that can no longer be downloaded can still be
# installed from a USB stick, and a system update brings newer entries.
#
# Only content that reads well in a text browser is listed: Wikimedia
# projects (mwoffliner), Stack Exchange (sotoki) and static websites
# (zimit). Collections that need JavaScript (zimgit PDF libraries, the
# Gutenberg index) are not.
let
  kiwix = dir: file: [ "https://download.kiwix.org/zim/${dir}/${file}" ];
  wikipedia = "Wikipedia contributors; packaged by Kiwix (download.kiwix.org)";
  medical = ''
    For information only. It does not replace a doctor, a pharmacist or
    emergency services. Articles may be out of date or wrong.'';
in
[
  # --- Encyclopedias -------------------------------------------------------
  {
    id = "wikipedia-en-top";
    title = "Wikipedia (English): top articles, text only";
    description = "The most-read English Wikipedia articles, without pictures. A good first pack.";
    category = "encyclopedia";
    language = "en";
    file = "wikipedia_en_top_nopic_2026-09.zim";
    size = 2745850723;
    sha256 = "8369b95b6b2179051b56ebed0d43fd3646020d03367eb03d7fc34d84090c501b";
    urls = kiwix "wikipedia" "wikipedia_en_top_nopic_2026-09.zim";
    license = "CC-BY-SA-4.0";
    source = wikipedia;
  }
  {
    id = "wikipedia-en-all-mini";
    title = "Wikipedia (English): all articles, introductions only";
    description = "Every English Wikipedia article, but only its introduction and infobox.";
    category = "encyclopedia";
    language = "en";
    file = "wikipedia_en_all_mini_2026-09.zim";
    size = 14386799064;
    sha256 = "350d8956ee3d7abe2839c464c3a983f93d6934a16c7f04b10715b49439dfd349";
    urls = kiwix "wikipedia" "wikipedia_en_all_mini_2026-09.zim";
    license = "CC-BY-SA-4.0";
    source = wikipedia;
  }
  {
    id = "wikipedia-en-all";
    title = "Wikipedia (English): all articles, text only";
    description = "The complete English Wikipedia without pictures. Very large: needs a big disk and a long download.";
    category = "encyclopedia";
    language = "en";
    file = "wikipedia_en_all_nopic_2026-06.zim";
    size = 52690706555;
    sha256 = "441a56d9e05b2d98f8ae9acb7986a513ed47904d73852c92dc6b7d50baa122e5";
    urls = kiwix "wikipedia" "wikipedia_en_all_nopic_2026-06.zim";
    license = "CC-BY-SA-4.0";
    source = wikipedia;
  }
  {
    id = "wikipedia-fr-top";
    title = "Wikipédia (français) : articles principaux, texte seul";
    description = "Les articles de Wikipédia en français les plus lus, sans images.";
    category = "encyclopedia";
    language = "fr";
    file = "wikipedia_fr_top_nopic_2026-10.zim";
    size = 1481903360;
    sha256 = "7b7da073fe62c73378719568555ed5bcbb484164e45f16e1a5051c8b5e6764ad";
    urls = kiwix "wikipedia" "wikipedia_fr_top_nopic_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = wikipedia;
  }
  {
    id = "wikipedia-fr-all";
    title = "Wikipédia (français) : tous les articles, texte seul";
    description = "Toute la Wikipédia en français, sans images. Volumineux.";
    category = "encyclopedia";
    language = "fr";
    file = "wikipedia_fr_all_nopic_2026-05.zim";
    size = 12912822846;
    sha256 = "c7c70f2af718455c3ff8a084c95395622d4c137c7fb0cf4f6bf122f362152b0c";
    urls = kiwix "wikipedia" "wikipedia_fr_all_nopic_2026-05.zim";
    license = "CC-BY-SA-4.0";
    source = wikipedia;
  }

  # --- Dictionaries --------------------------------------------------------
  {
    id = "wiktionary-en";
    title = "Wiktionary (English): dictionary, text only";
    description = "Definitions, translations and pronunciations of words in many languages, in English.";
    category = "dictionary";
    language = "en";
    file = "wiktionary_en_all_nopic_2026-11.zim";
    size = 14847562421;
    sha256 = "a2909d79f62ae05b644310566d68e766100fe7cdd5a061b0e4f2a585c853b8fb";
    urls = kiwix "wiktionary" "wiktionary_en_all_nopic_2026-11.zim";
    license = "CC-BY-SA-4.0";
    source = "Wiktionary contributors; packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "wiktionary-fr";
    title = "Wiktionnaire (français) : dictionnaire, texte seul";
    description = "Définitions, traductions et prononciations, en français.";
    category = "dictionary";
    language = "fr";
    file = "wiktionary_fr_all_nopic_2026-08.zim";
    size = 4935856722;
    sha256 = "d44f79ef836f8bbc464f9369e1b8b8bdfced95968b42d15813579975c03f785b";
    urls = kiwix "wiktionary" "wiktionary_fr_all_nopic_2026-08.zim";
    license = "CC-BY-SA-4.0";
    source = "Wiktionnaire contributors; packaged by Kiwix (download.kiwix.org)";
  }

  # --- Health --------------------------------------------------------------
  {
    id = "wikipedia-en-medicine";
    title = "Wikipedia (English): medicine, text only";
    description = "Wikipedia's articles on health, diseases, anatomy and medicines.";
    category = "health";
    language = "en";
    file = "wikipedia_en_medicine_nopic_2026-10.zim";
    size = 1047708415;
    sha256 = "718fadad40b3491be3fa8f95181a948e4d726f97a79a14e0ef1b90c6b2b03625";
    urls = kiwix "wikipedia" "wikipedia_en_medicine_nopic_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = wikipedia;
    notice = medical;
  }
  {
    id = "wikipedia-fr-medicine";
    title = "Wikipédia (français) : médecine, texte seul";
    description = "Les articles de Wikipédia sur la santé, les maladies et les médicaments.";
    category = "health";
    language = "fr";
    file = "wikipedia_fr_medicine_nopic_2026-10.zim";
    size = 320851596;
    sha256 = "f10857e0d3d0bcfa8db4e106baf52e77f7c76b124a0f61505711b4e98402f592";
    urls = kiwix "wikipedia" "wikipedia_fr_medicine_nopic_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = wikipedia;
    notice = medical;
  }
  {
    id = "mdwiki-en";
    title = "WikiProject Medicine (mdwiki, English)";
    description = "Medical articles reviewed by WikiProject Medicine, with pictures.";
    category = "health";
    language = "en";
    file = "mdwiki_en_all_maxi_2025-11.zim";
    size = 2302838768;
    sha256 = "ee51df7860dbcab8c628aad32cf9972726ab708d784cdaf3e217d596920edf80";
    urls = kiwix "other" "mdwiki_en_all_maxi_2025-11.zim";
    license = "CC-BY-SA-4.0";
    source = "mdwiki.org contributors; packaged by Kiwix (download.kiwix.org)";
    notice = medical;
  }
  {
    id = "nhs-medicines";
    title = "NHS medicines A to Z (English)";
    description = "The UK National Health Service's guide to common medicines: uses, doses, side effects.";
    category = "health";
    language = "en";
    file = "nhs.uk_en_medicines_2025-12.zim";
    size = 16946183;
    sha256 = "7dfa9bffa286e1ed4442536e746109a58d678bc6681af0b9881e07b7ef3dae28";
    urls = kiwix "zimit" "nhs.uk_en_medicines_2025-12.zim";
    license = "OGL-UK-3.0";
    source = "NHS website (nhs.uk); packaged by Kiwix (download.kiwix.org)";
    notice = medical;
  }

  # --- Practical -----------------------------------------------------------
  {
    id = "ready-gov";
    title = "Ready.gov: emergency preparedness (English)";
    description = "US government guides to prepare for and get through disasters and emergencies.";
    category = "practical";
    language = "en";
    file = "ready.gov_en_all_2026-10.zim";
    size = 2063895408;
    sha256 = "c88ce03db2a6dd855c1075d87abb6dceac678718e0dd3fa7912009469b9ca225";
    urls = kiwix "zimit" "ready.gov_en_all_2026-10.zim";
    license = "LicenseRef-US-Government-Work";
    source = "ready.gov (US FEMA); packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "appropedia";
    title = "Appropedia: sustainability and appropriate technology";
    description = "Practical solutions for water, energy, food, shelter and health, in English.";
    category = "practical";
    language = "en";
    file = "appropedia_en_all_maxi_2026-02.zim";
    size = 581641351;
    sha256 = "dcf200ba723c27397e1c9a2e4891ff91fa13993df1fa285315190d8525a53469";
    urls = kiwix "other" "appropedia_en_all_maxi_2026-02.zim";
    license = "CC-BY-SA-4.0";
    source = "Appropedia contributors; packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "energypedia";
    title = "Energypedia: renewable energy, text only";
    description = "Solar, wind, hydro and cooking energy in practice, in English.";
    category = "practical";
    language = "en";
    file = "energypedia_en_all_nopic_2026-06.zim";
    size = 722590405;
    sha256 = "f1b2cf0afde2927eab9acd2b6365144eae35986894c2a8eac7071045fd01d295";
    urls = kiwix "other" "energypedia_en_all_nopic_2026-06.zim";
    license = "CC-BY-SA-4.0";
    source = "Energypedia contributors; packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "se-gardening";
    title = "Gardening & Landscaping questions (Stack Exchange)";
    description = "Questions and answers about growing plants, vegetables and trees, in English.";
    category = "practical";
    language = "en";
    file = "gardening.stackexchange.com_en_all_2026-10.zim";
    size = 933097375;
    sha256 = "8079b77ba40d830c31cdfa0e3f43a100e9d9488d856b10ffb3aca4e829594e6e";
    urls = kiwix "stack_exchange" "gardening.stackexchange.com_en_all_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = "Stack Exchange contributors; packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "se-outdoors";
    title = "The Great Outdoors questions (Stack Exchange)";
    description = "Questions and answers about camping, hiking, navigation and survival outdoors, in English.";
    category = "practical";
    language = "en";
    file = "outdoors.stackexchange.com_en_all_2026-10.zim";
    size = 141820374;
    sha256 = "73752057bd2253af6cf80c69caf0def158bcf378188f0fbc7ab8ed07bbbe9bc5";
    urls = kiwix "stack_exchange" "outdoors.stackexchange.com_en_all_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = "Stack Exchange contributors; packaged by Kiwix (download.kiwix.org)";
  }

  # --- Travel --------------------------------------------------------------
  {
    id = "wikivoyage-en";
    title = "Wikivoyage (English): travel guide, text only";
    description = "Places to go, getting around, staying safe, for the whole world.";
    category = "travel";
    language = "en";
    file = "wikivoyage_en_all_nopic_2026-09.zim";
    size = 272495142;
    sha256 = "4305c50550f065d83e70f75fd1d2016a694f5d67aa1e0ef00fdf6bb74fc092ac";
    urls = kiwix "wikivoyage" "wikivoyage_en_all_nopic_2026-09.zim";
    license = "CC-BY-SA-4.0";
    source = "Wikivoyage contributors; packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "wikivoyage-fr";
    title = "Wikivoyage (français) : guide de voyage, texte seul";
    description = "Destinations, transports et sécurité, pour le monde entier.";
    category = "travel";
    language = "fr";
    file = "wikivoyage_fr_all_nopic_2026-10.zim";
    size = 68397127;
    sha256 = "c25c4c616f4c283508d8fea1c4b8ef9f1df8ae7d3f72354e608be97f645356ad";
    urls = kiwix "wikivoyage" "wikivoyage_fr_all_nopic_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = "Wikivoyage contributors; packaged by Kiwix (download.kiwix.org)";
  }

  # --- Computing -----------------------------------------------------------
  {
    id = "archlinux-wiki";
    title = "ArchWiki: Linux documentation (English)";
    description = "Detailed, practical documentation about Linux programs and hardware.";
    category = "computing";
    language = "en";
    file = "archlinux_en_all_maxi_2026-10.zim";
    size = 38311014;
    sha256 = "e0cabb299ce29d7a60e0a65cab4d3e14d8e60b5995daeb60b2932bbac04e505b";
    urls = kiwix "other" "archlinux_en_all_maxi_2026-10.zim";
    license = "GFDL-1.3-or-later";
    source = "ArchWiki contributors; packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "se-unix";
    title = "Unix & Linux questions (Stack Exchange)";
    description = "Questions and answers about the Linux command line, shells and system administration.";
    category = "computing";
    language = "en";
    file = "unix.stackexchange.com_en_all_2026-10.zim";
    size = 1316253305;
    sha256 = "793f0fbaf64e9eb8d2fd59fec80f62cf94acb22ca94c87eb25ae145dd589e551";
    urls = kiwix "stack_exchange" "unix.stackexchange.com_en_all_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = "Stack Exchange contributors; packaged by Kiwix (download.kiwix.org)";
  }
  {
    id = "se-superuser";
    title = "Super User questions (Stack Exchange)";
    description = "Questions and answers about using and repairing computers.";
    category = "computing";
    language = "en";
    file = "superuser.com_en_all_2026-10.zim";
    size = 4014454341;
    sha256 = "e2392c0ccb7627740c4bd8e922d41b5d592da35ab4985f89d833298dc7e92f76";
    urls = kiwix "stack_exchange" "superuser.com_en_all_2026-10.zim";
    license = "CC-BY-SA-4.0";
    source = "Stack Exchange contributors; packaged by Kiwix (download.kiwix.org)";
  }
]
