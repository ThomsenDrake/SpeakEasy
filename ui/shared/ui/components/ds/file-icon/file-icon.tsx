import { cn } from "../../../utils/cn";
import { Default } from "./assets/default.tsx";
import { Image } from "./assets/image.tsx";
import { Music } from "./assets/music.tsx";
import { Pdf } from "./assets/pdf.tsx";
import { Spreadsheet } from "./assets/spreadsheet.tsx";
import { TextDocument } from "./assets/text-document.tsx";
import { Video } from "./assets/video.tsx";
import { Web } from "./assets/web.tsx";

const FILE_ICONS = [
  { extension: "jpg", icon: Image },
  { extension: "jpeg", icon: Image },
  { extension: "png", icon: Image },
  { extension: "pdf", icon: Pdf },
  { extension: "txt", icon: TextDocument },
  { extension: "docx", icon: TextDocument },
  { extension: "xlsx", icon: Spreadsheet },
  { extension: "csv", icon: Spreadsheet },
  { extension: "bmp", icon: Image },
  { extension: "webp", icon: Image },
  { extension: "mp3", icon: Music },
  { extension: "wav", icon: Music },
  { extension: "ogg", icon: Music },
  { extension: "m4a", icon: Music },
  { extension: "flac", icon: Music },
  { extension: "mp4", icon: Video },
  { extension: "webm", icon: Video },
  { extension: "mov", icon: Video },
  { extension: "mkv", icon: Video },
];

const GenericFileIcon = ({
  extension,
  className,
}: {
  extension: string | null;
  className?: string;
}) => {
  const displayExtension = extension?.toUpperCase() ?? "FILE";
  return (
    <div className={cn("relative", className)}>
      <Default />
      {/* position the file extension just right on the small icon badge hehe */}
      <div className="absolute inset-0 grid grid-cols-[1fr] grid-rows-[2.35fr_auto_1fr] ps-[2px]">
        <span />
        <span
          className={cn(
            "text-white-default row-start-2 row-end-3 font-medium",
            displayExtension.length > 3 ? "text-[6px]" : "text-[7px]",
          )}
        >
          {displayExtension}
        </span>
      </div>
    </div>
  );
};

export const DocumentIcon = ({
  extension,
  url,
  className,
}: {
  extension: string | null;
  url?: string | null;
  className?: string;
}) => {
  if (url) {
    return <Web className={cn("size-8", className)} />;
  }
  const Icon = FILE_ICONS.find((icon) => icon.extension === extension)?.icon;

  if (Icon) {
    return <Icon className={cn("size-8", className)} />;
  }

  return (
    <GenericFileIcon
      extension={extension}
      className={cn("size-8", className)}
    />
  );
};
