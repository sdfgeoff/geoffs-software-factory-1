import { memo, useEffect, useId, useMemo, useRef, type ChangeEvent } from "react";
import type { Attachment } from "../types";

export interface PendingImage {
  key: string;
  file: File;
  attachment?: Attachment;
  state: "selected" | "uploading" | "uploaded" | "error";
  error?: string;
}

let nextImageKey = 0;

export function pendingImages(files: File[]): PendingImage[] {
  return files.map((file) => ({
    key: `image-${nextImageKey += 1}`,
    file,
    state: "selected",
  }));
}

const LocalImage = memo(function LocalImage({ file }: { file: File }) {
  const url = useMemo(() => URL.createObjectURL(file), [file]);
  useEffect(() => () => URL.revokeObjectURL(url), [url]);
  return <img src={url} alt={`Preview of ${file.name}`} />;
});

export function ImageAttachmentPicker({
  images,
  disabled,
  onAdd,
  onRemove,
}: {
  images: PendingImage[];
  disabled: boolean;
  onAdd: (images: PendingImage[]) => void;
  onRemove: (key: string) => void;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const inputId = useId();
  function selected(event: ChangeEvent<HTMLInputElement>) {
    const files = [...(event.target.files ?? [])].filter((file) => file.type.startsWith("image/"));
    if (files.length) onAdd(pendingImages(files));
    event.target.value = "";
  }
  return (
    <div className="image-attachment-picker">
      <input
        ref={inputRef}
        id={inputId}
        className="visually-hidden"
        type="file"
        accept="image/*"
        multiple
        disabled={disabled}
        onChange={selected}
      />
      <label className="button-link secondary attach-images" htmlFor={inputId} aria-disabled={disabled}>
        Attach images
      </label>
      {images.length > 0 && (
        <div className="pending-images" aria-label="Selected images">
          {images.map((image) => (
            <figure key={image.key}>
              <LocalImage file={image.file} />
              <figcaption title={image.file.name}>{image.file.name}</figcaption>
              <small>{image.state === "uploading" ? "Uploading…" : image.state === "uploaded" ? "Uploaded" : image.error ?? "Ready"}</small>
              <button
                type="button"
                className="ghost"
                disabled={disabled}
                aria-label={`Remove ${image.file.name}`}
                onClick={() => onRemove(image.key)}
              >
                Remove
              </button>
            </figure>
          ))}
        </div>
      )}
    </div>
  );
}

export const AttachmentGallery = memo(function AttachmentGallery({
  attachments,
}: {
  attachments: Attachment[];
}) {
  if (!attachments.length) return null;
  return (
    <div className="message-attachments" aria-label="Attached images">
      {attachments.map((attachment) => (
        <figure key={attachment.id}>
          <a href={attachment.download_url} target="_blank" rel="noreferrer" aria-label={`Open ${attachment.original_filename} full size`}>
            <img
              src={attachment.download_url}
              alt={attachment.original_filename}
              loading="lazy"
            />
          </a>
          <figcaption>
            <span title={attachment.original_filename}>{attachment.original_filename}</span>
            <a href={attachment.download_url} download={attachment.original_filename}>Download</a>
          </figcaption>
        </figure>
      ))}
    </div>
  );
});
