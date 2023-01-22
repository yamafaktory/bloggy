module.exports = {
  content: ['./public/**/*.html'],
  theme: {
    extend: {
      colors: {
        core: '#111010',
      },
      fontFamily: {
        jetbrains: ['JetBrains Mono', 'monospace'],
      },
    },
  },
  plugins: [require('@tailwindcss/forms')],
}
