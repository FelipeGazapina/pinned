function routeAnchorClick(link) {
  if (!link || link.inPanel || link.download || link.pr || link.cardPr) return null;
  if (link.openUrl) return { openOutside: link.openUrl };
  if (link.protocol !== "http:" && link.protocol !== "https:") return null;
  return { url: link.href, outside: !!(link.meta || link.ctrl) };
}
