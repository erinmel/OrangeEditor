function normalize(value) {
  return value.toLowerCase().normalize('NFKD').replace(/\p{M}/gu, '')
    .replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
}

export function prepareIndex(entries) {
  return entries.map(entry => ({
    ...entry,
    titleKey: normalize(entry.title),
    aliasKey: normalize(entry.alias ?? ''),
    summaryKey: normalize(entry.description),
    bodyKey: normalize(entry.terms),
  }));
}

export function searchIndex(index, input) {
  const query = normalize(input);
  if (!query) return [];
  const words = query.split(' ');
  const matches = [];
  for (const entry of index) {
    const {titleKey, aliasKey, summaryKey, bodyKey} = entry;
    if (!words.every(word => titleKey.includes(word) || aliasKey.includes(word) ||
        summaryKey.includes(word) || bodyKey.includes(word))) continue;
    const score = (titleKey === query ? 100 : 0) + (aliasKey === query ? 95 : 0) +
      (titleKey.startsWith(query) ? 40 : 0) + (aliasKey.startsWith(query) ? 35 : 0) +
      (summaryKey.startsWith(query + ' ') ? 25 : 0) + (summaryKey.includes(query) ? 15 : 0) +
      words.filter(word => titleKey.includes(word) || aliasKey.includes(word)).length * 8;
    matches.push({entry, score});
  }
  matches.sort((a, b) => b.score - a.score);
  return matches.map(({entry}) => entry);
}
