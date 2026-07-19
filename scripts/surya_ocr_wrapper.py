# =============================================================================
# PDF Lks Text Editor - Script wrapper Surya OCR
# Créé par Leksa667 (https://github.com/Leksa667)
#
# Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
# et le distribuer librement, à condition de créditer l'auteur original.
# Aucune garantie n'est fournie.
# =============================================================================

import json, sys

from PIL import Image
from surya.detection import DetectionPredictor
from surya.recognition import RecognitionPredictor

image_path = sys.argv[1]
langs = sys.argv[2].split(",")

img = Image.open(image_path)
det = DetectionPredictor()
rec = RecognitionPredictor()
results = rec([img], [langs], det_predictor=det)

out = []
for page in results:
    lines = []
    for tl in page.text_lines:
        lines.append({
            "text": tl.text,
            "bbox": tl.bbox,
            "confidence": tl.confidence,
        })
    out.append({
        "text_lines": lines,
        "image_bbox": page.image_bbox,
    })

print(json.dumps(out, ensure_ascii=False))
