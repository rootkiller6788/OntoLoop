export const UI = {
  println(...parts) {
    process.stdout.write(parts.join(" ") + "\n");
  },
  error(message) {
    process.stderr.write(`${message}\n`);
  },
};
