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
  if (matches.length) return matches.map(({entry}) => entry);

  // Keep typo matching on short words in titles and aliases. Body text is too broad.
  if (!words.some(word => word.length >= 4)) return [];
  const scoreWords = key => {
    const tokens = key.split(' ');
    let score = 0;
    for (const word of words) {
      if (tokens.some(token => token.startsWith(word))) score += 2;
      else if (word.length >= 4 && tokens.some(token => oneEdit(word, token))) score += 1;
      else return 0;
    }
    return score;
  };
  return index.map(entry => ({entry, score: Math.max(scoreWords(entry.titleKey) * 2, scoreWords(entry.aliasKey))}))
    .filter(match => match.score > 0).sort((a, b) => b.score - a.score).map(match => match.entry);
}

function oneEdit(word, token) {
  if (Math.abs(word.length - token.length) > 1) return false;
  const boundary = Math.min(word.length, token.length);
  let at = 0;
  while (at < boundary && word[at] === token[at]) at++;
  if (word.length > token.length) return word.slice(at + 1) === token.slice(at);
  if (token.length > word.length) return word.slice(at) === token.slice(at + 1);
  return word.slice(at + 1) === token.slice(at + 1) ||
    (word[at] === token[at + 1] && token[at] === word[at + 1] && word.slice(at + 2) === token.slice(at + 2));
}
