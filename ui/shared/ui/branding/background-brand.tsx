export const BackgroundBrand = () => (
  <svg
    width="100%"
    height="100%"
    className="absolute top-0 left-0 z-0"
    style={{
      background: "linear-gradient(to bottom, #FA500F, #FFAF00)",
    }}
  >
    <defs>
      <pattern
        id="smallSquares"
        width="20"
        height="20"
        patternUnits="userSpaceOnUse"
      >
        <rect x="5" y="5" width="2" height="2" fill="white" fillOpacity="0.5" />
      </pattern>
    </defs>
    <rect width="100%" height="100%" fill="url(#smallSquares)" />
  </svg>
);
