import uno
from com.sun.star.beans import PropertyValue

def pv(name, value):
    p = PropertyValue()
    p.Name = name
    p.Value = value
    return p

localContext = uno.getComponentContext()
resolver = localContext.ServiceManager.createInstanceWithContext('com.sun.star.bridge.UnoUrlResolver', localContext)
ctx = resolver.resolve('uno:socket,host=localhost,port=2002;urp;StarOffice.ComponentContext')
smgr = ctx.ServiceManager
desktop = smgr.createInstanceWithContext('com.sun.star.frame.Desktop', ctx)

doc = desktop.loadComponentFromURL('private:factory/simpress', '_blank', 0, (pv('Hidden', True),))
slides = doc.DrawPages
slides.insertNewByIndex(1)  # now 2 slides total

# --- Slide 1: a styled text box with mixed run formatting + a colored page background ---
s1 = slides.getByIndex(0)
s1.Layout = 20  # blank layout

shape = doc.createInstance('com.sun.star.presentation.TitleTextShape') if False else doc.createInstance('com.sun.star.drawing.TextShape')
s1.add(shape)
from com.sun.star.awt import Point, Size
shape.Position = Point(2000, 1500)   # 1/100 mm
shape.Size = Size(16000, 3000)
shape.TextAutoGrowHeight = False
shape.TextAutoGrowWidth = False

text = shape.getText()
text.setString("")
cursor = text.createTextCursor()
cursor.ParaAdjust = uno.Enum("com.sun.star.style.ParagraphAdjust", "RIGHT")
text.insertString(cursor, "Hello ", False)
# Use direct float constants instead (FontWeight.BOLD = 150.0, FontSlant.ITALIC = enum)
import uno as _uno
text.insertString(cursor, "World", False)
# Apply bold+color+font+size to the "World" run only: re-select it
sel_cursor = text.createTextCursorByRange(cursor.End)
# Simplify: format whole second run by creating cursor across just-inserted text
cursor2 = text.createTextCursor()
cursor2.gotoStart(False)
cursor2.goRight(6, False)   # past "Hello "
cursor2.goRight(5, True)    # select "World"
cursor2.CharWeight = 150.0  # com.sun.star.awt.FontWeight.BOLD
cursor2.CharColor = 0xFF0000
cursor2.CharHeight = 28.0
cursor2.CharFontName = "Liberation Sans"
cursor2.CharPosture = 2  # com.sun.star.awt.FontSlant.ITALIC
text.insertString(text.createTextCursorByRange(text.End), " plain", False)

# second paragraph
text.insertControlCharacter(text.End, 0, False)
text.insertString(text.End, "Second paragraph left", False)
cur3 = text.createTextCursor()
cur3.gotoEnd(False)
cur3.goLeft(len("Second paragraph left"), True)
cur3.ParaAdjust = uno.Enum("com.sun.star.style.ParagraphAdjust", "LEFT")


# --- Slide 2: a rectangle shape + an image ---
s2 = slides.getByIndex(1)
s2.Layout = 20
rect = doc.createInstance('com.sun.star.drawing.RectangleShape')
s2.add(rect)
rect.Position = Point(1000, 1000)
rect.Size = Size(5000, 3000)
rect.FillColor = 0x00AA00

img = doc.createInstance('com.sun.star.drawing.GraphicObjectShape')
s2.add(img)
img.Position = Point(3000, 2000)
img.Size = Size(4000, 4000)
graphic_provider = smgr.createInstanceWithContext('com.sun.star.graphic.GraphicProvider', ctx)
img_url = 'file:///home/jasonb/.claude/jobs/0e8fb149/tmp/odp/test.png'
graphic = graphic_provider.queryGraphic((pv('URL', img_url),))
img.Graphic = graphic

text2 = s2
tb2 = doc.createInstance('com.sun.star.drawing.TextShape')
s2.add(tb2)
tb2.Position = Point(500, 500)
tb2.Size = Size(8000, 1500)
tb2.getText().setString("Second slide title")

out_url = 'file:///home/jasonb/.claude/jobs/0e8fb149/tmp/odp/fixture.odp'
doc.storeToURL(out_url, (pv('FilterName', 'impress8'),))
doc.close(False)
print("DONE")
