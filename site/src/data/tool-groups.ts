export const toolGroups = [
  {
    id: 'image',
    title: 'Image editing tools',
    shots: [
      { id: 'remove-bg', alt: 'The OpenConvert image workspace with a subject cut out of its background onto transparency.' },
      { id: 'upscale', alt: 'The same workspace enlarging an image four times on device.' },
      { id: 'colour-picker', alt: 'The colour picker sampling a value straight out of the image.' },
    ],
    items: [
      'Remove background: on-device matting, nothing uploaded, different AI models available',
      'Upscale x2 or x4 without the usual blur using a local AI model',
      'Invert colours, black and white',
      'Compress to the quality you pick, without changing the format',
      'Read the text out of a screenshot or a scan',
      'Colour picker, sampled from the image itself',
      'Convert between HEIC, JPEG, PNG, WebP, AVIF, TIFF, JPEG XL, SVG and camera raw',
    ],
  },
  {
    id: 'pdf',
    title: 'PDF tools',
    shots: [
      { id: 'pdf-merge', alt: 'Two PDFs loaded in OpenConvert, ready to merge into one file.' },
      { id: 'pdf-reorder', alt: 'A document’s pages listed so they can be moved or dropped before the file is written.' },
    ],
    items: [
      'Merge documents into one file while keeping hyperlinks, or split one into many',
      'Keep, drop, reorder, rotate or crop pages',
      'Add a password, or remove one you know',
      'Compress without touching a single image',
      'Sign, with the signature kept on your machine',
      'Render any page to an image',
      'Read a scan into searchable text',
      'PDF to Markdown, HTML, DOCX or ODT',
    ],
  },
  {
    id: 'audio',
    title: 'Audio tools',
    shots: [
      { id: 'transcribe', alt: 'A recording and the transcript OpenConvert produced from it locally.' },
      { id: 'denoise', alt: 'The same recording with its background noise removed on this machine.' },
    ],
    items: [
      'Transcribe speech into text you can search',
      'Remove background noise from a recording',
      'Convert between FLAC, WAV, MP3, OGG and M4A',
      'Pull the audio out of a video without re-encoding it',
    ],
  },
];
