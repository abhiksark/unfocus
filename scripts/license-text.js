export function normalizeLicenseText(text) {
  return text.replaceAll("\r\n", "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .trim() + "\n";
}
