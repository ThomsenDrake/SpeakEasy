import { LoaderCircle } from "lucide-react";

export function LoadingSpinnerCentered() {
  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex flex-1 items-center justify-center">
        <div className="flex items-center">
          <LoaderCircle className="text-muted h-6 w-6 animate-spin" />
        </div>
      </div>
    </div>
  );
}
