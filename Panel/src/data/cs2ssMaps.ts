const MAP_NAMES: Record<string, string> = {
  "de_dust2": "Dust II",
  "de_anubis": "Anubis",
  "de_inferno": "Inferno",
  "de_mirage": "Mirage",
  "de_nuke": "Nuke",
  "de_vertigo": "Vertigo",
  "de_ancient": "Ancient",
  "de_overpass": "Overpass",
  "cs_office": "Office",
  "cs_italy": "Italy",
  "de_train": "Train",
  "de_cache": "Cache",
  "de_cbble": "Cobblestone",
  "de_canals": "Canals",
  "de_basalt": "Basalt",
  "de_iris": "Iris",
  "de_thera": "Thera",
  "de_whistle": "Whistle",
  "de_memento": "Memento",
  "de_edin": "Edin",
  "de_palais": "Palais",
  "de_mills": "Mills",
  "de_assembly": "Assembly",
  "ar_shoots": "Shoots",
  "ar_baggage": "Baggage",
};

export function cs2ssMapLabel(name: string): string {
  return MAP_NAMES[name] ?? name;
}