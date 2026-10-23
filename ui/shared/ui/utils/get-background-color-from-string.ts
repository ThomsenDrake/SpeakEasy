const COLORS = [
  "bg-basic-yellow-accent",
  "bg-basic-orange-accent",
  "bg-basic-green-accent",
  "bg-basic-red-accent",
  "bg-basic-purple-accent",
  "bg-basic-pink-accent",
  "bg-basic-teal-accent",
  "bg-basic-emerald-accent",
  "bg-basic-lime-accent",
  "bg-basic-cyan-accent",
  "bg-basic-sky-accent",
  "bg-basic-blue-accent",
  "bg-basic-bluedeep-accent",
  "bg-basic-purple-accent",
];

export function getBackgroundColorFromString(str: string) {
  let seed = 0;

  for (const char of str) {
    seed += char.charCodeAt(0);
  }

  return COLORS[seed % COLORS.length];
}
