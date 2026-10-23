interface CustomColorLogoProps {
  color?: string;
}

export const CustomColorLogo = ({
  color = "text-white-default",
}: CustomColorLogoProps) => {
  return (
    <svg
      width="80"
      height="57"
      viewBox="0 0 80 57"
      fill="none"
      className={color}
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M45.7148 45.1582H57.1436V33.8691H45.7139L45.7148 45.1582H34.2852V33.8691H22.8574V45.1582H34.2852V56.4482H0V45.1582H11.4287V0H22.8574V11.29H34.2861V22.5791H45.7148V11.29H57.1436V0H68.5723V45.1582H80V56.4482H45.7148V45.1582Z"
        fill="currentColor"
      />
    </svg>
  );
};
