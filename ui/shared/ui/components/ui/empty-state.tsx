import { Typography } from "#components/ds/typography/index.ts";
import { cn } from "#utils/cn.ts";

import { DitheringAnimation } from "./dithering-animation";

interface EmptyStateProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  children?: React.ReactNode;
  icon?: React.ReactNode;
  isError?: boolean;
  contentClassName?: string;
  containerClassName?: string;
  iconClassName?: string;
}

export const EmptyState = ({
  title,
  description,
  icon,
  isError,
  children,
  contentClassName,
  containerClassName,
  iconClassName,
}: EmptyStateProps) => {
  const iconBoxShadow =
    "0 -1px 0 0 rgba(0, 0, 0, 0.10) inset, 0 1px 2px 0 rgba(0, 0, 0, 0.05)";

  return (
    <div
      id="empty-state"
      className={cn(
        "bg-subtle relative flex h-full w-full overflow-hidden",
        {
          "bg-[#fffafa]": isError,
        },
        containerClassName,
      )}
    >
      {
        // eslint-disable-next-line n/no-process-env
        process.env.NEXT_PUBLIC_STATIC_BACKGROUND !== "true" && (
          <DitheringAnimation
            colorFront={isError ? "#fff0f0" : "#F1F1F1"}
            colorBack={isError ? "#fffafa" : "#F7F6F3"}
          />
        )
      }

      <div className="absolute inset-0">
        <div
          className={cn(
            "flex h-full w-full flex-col items-center justify-center gap-4 px-6 transition-all duration-300",
            contentClassName,
          )}
        >
          {icon && (
            <div
              className={cn(
                "border-default bg-default flex size-16 items-center justify-center rounded-md border p-2",
                iconClassName,
              )}
              style={{ boxShadow: iconBoxShadow }}
            >
              {icon}
            </div>
          )}

          <div className="flex flex-col gap-2 text-center">
            <Typography size="lg" weight="semibold">
              {title}
            </Typography>

            {description ? (
              typeof description === "string" ? (
                <Typography
                  size="sm"
                  variant="subtle"
                  className="max-w-[500px]"
                >
                  {description}
                </Typography>
              ) : (
                <div className="max-w-[500px]">{description}</div>
              )
            ) : null}
          </div>

          {children}
        </div>
      </div>
    </div>
  );
};
