import { cn } from "../../utils";

interface TwoLineDateProps {
  date: Date | string;
  className?: string;
}

export const TwoLineDate = ({ date, className }: TwoLineDateProps) => {
  const dateObj = typeof date === "string" ? new Date(date) : date;
  const dateString = dateObj.toLocaleDateString();
  const timeString = dateObj.toLocaleTimeString();

  return (
    <div className={cn("text-muted text-center text-xs", className)}>
      <div>{dateString}</div>
      <div>{timeString}</div>
    </div>
  );
};
