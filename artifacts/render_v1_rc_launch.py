from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFont


W, H = 1080, 1350
ROOT = Path(__file__).resolve().parent
OUT = ROOT / "axiom-v1-rc-launch.png"
BLACK = "#0A0909"
PAPER = "#EEEAE2"
RED = "#E83B42"
WHITE = "#FAF7F0"
GREY = "#A9A39B"
INK = "#191616"
SANS = Path(r"C:\Windows\Fonts\segoeui.ttf")
BOLD = Path(r"C:\Windows\Fonts\segoeuib.ttf")
DISPLAY = Path(r"C:\Windows\Fonts\bahnschrift.ttf")
MONO = Path(r"C:\Windows\Fonts\CascadiaMono.ttf")


def face(path: Path, size: int):
    return ImageFont.truetype(str(path), size=size)


def spaced(draw, xy, value, font, fill, spacing):
    x, y = xy
    for char in value:
        draw.text((x, y), char, font=font, fill=fill)
        x += draw.textlength(char, font=font) + spacing


def spans(draw, xy, parts, font):
    x, y = xy
    for value, color in parts:
        draw.text((x, y), value, font=font, fill=color)
        x += draw.textlength(value, font=font)


image = Image.new("RGB", (W, H), PAPER)
noise = Image.effect_noise((W, H), 5).convert("L")
noise = ImageEnhance.Contrast(noise).enhance(0.65)
grain = Image.merge("RGB", (noise, noise, noise))
image = ImageChops.soft_light(image, grain)
draw = ImageDraw.Draw(image)

mono_15 = face(MONO, 15)
mono_17 = face(MONO, 17)
mono_20 = face(MONO, 20)
sans_24 = face(SANS, 24)
sans_30 = face(SANS, 30)
sans_46 = face(BOLD, 46)
display_104 = face(DISPLAY, 104)
display_224 = face(DISPLAY, 224)

draw.rectangle((0, 0, W, 10), fill=RED)
spaced(draw, (58, 50), "NEXARA AI / AXIOM", mono_15, INK, 1.2)
spaced(draw, (817, 50), "RC 01 / 2026", mono_15, INK, 1.2)
draw.line((58, 95, 1022, 95), fill="#B8B1A8", width=1)

draw.rectangle((58, 130, 1022, 648), fill=BLACK)
draw.rectangle((58, 130, 72, 648), fill=RED)
spaced(draw, (96, 164), "AXIOM", display_104, WHITE, -1.5)
draw.text((84, 260), "V1", font=display_224, fill=RED, stroke_width=1, stroke_fill=RED)
spaced(draw, (765, 183), "RELEASE", mono_17, WHITE, 1.5)
spaced(draw, (765, 211), "CANDIDATE", mono_17, WHITE, 1.5)
draw.line((765, 260, 958, 260), fill="#514C4A", width=1)
draw.text((765, 285), "Built for the", font=sans_24, fill=GREY)
draw.text((765, 319), "work between", font=sans_24, fill=GREY)
draw.text((765, 353), "idea and proof.", font=sans_24, fill=WHITE)
draw.text((96, 548), "The coding agent built to prove every action.", font=sans_30, fill=WHITE)

draw.text((58, 705), "Proof is part of the product.", font=sans_46, fill=INK)
draw.text((60, 770), "Plans, tools, patches, tests and results stay visible", font=sans_24, fill="#4A4541")
draw.text((60, 805), "from the first prompt to the final answer.", font=sans_24, fill="#4A4541")

draw.rectangle((58, 858, 1022, 1135), fill=BLACK)
draw.rectangle((58, 858, 1022, 902), fill="#171414")
draw.rectangle((58, 858, 176, 902), fill=RED)
spaced(draw, (78, 871), "TERMINAL", mono_15, WHITE, 1.1)
spaced(draw, (800, 871), "AXIOM / RC.1", mono_15, GREY, 1.0)

spans(draw, (86, 928), [("$ ", RED), ("npm install -g axiom-agent@rc", WHITE)], mono_20)
spans(draw, (86, 970), [("$ ", RED), ("axiom", WHITE)], mono_20)
spans(draw, (86, 1024), [("Axiom Lens: ", RED), ("selected file.read, project.scan", WHITE)], mono_20)
spans(draw, (86, 1062), [("Axiom Tool: ", "#4ED29A"), ("executed file.read", WHITE)], mono_20)
spans(draw, (86, 1100), [("Result ", RED), ("verified and summarized.", WHITE)], mono_20)

spaced(draw, (58, 1177), "NATIVE TOOLS", mono_15, INK, 1.0)
draw.rectangle((251, 1183, 258, 1190), fill=RED)
spaced(draw, (288, 1177), "SAFE PATCHES", mono_15, INK, 1.0)
draw.rectangle((474, 1183, 481, 1190), fill=RED)
spaced(draw, (511, 1177), "RESUMABLE", mono_15, INK, 1.0)
draw.rectangle((683, 1183, 690, 1190), fill=RED)
spaced(draw, (720, 1177), "PROOF TRAILS", mono_15, INK, 1.0)

draw.rectangle((0, 1240, W, H), fill=RED)
spaced(draw, (58, 1261), "AVAILABLE NOW", mono_17, WHITE, 1.3)
draw.text((58, 1293), "GitHub + npm", font=sans_24, fill=WHITE)
right = "© 2026 DemonZDevelopment"
right_width = draw.textlength(right, font=mono_15)
draw.text((1022 - right_width, 1298), right, font=mono_15, fill=WHITE)

image.save(OUT, format="PNG", optimize=True)
print(OUT)
