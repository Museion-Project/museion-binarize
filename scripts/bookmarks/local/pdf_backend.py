"""PDFium reading/rendering in visible top-left page points; pypdf metadata.

This is a distinct extraction path. Its measurements must not be relabelled as
historical MuPDF evidence. No OCR, network access, or source mutation.
"""
from dataclasses import dataclass
import ctypes
import pypdfium2 as pdfium
from pypdf import PdfReader

RENDERER = 'pdfium-' + str(pdfium.PDFIUM_INFO)

@dataclass(frozen=True)
class Rect:
    x0: float
    y0: float
    x1: float
    y1: float
    @property
    def width(self): return self.x1-self.x0
    @property
    def height(self): return self.y1-self.y0
    def __iter__(self): return iter((self.x0,self.y0,self.x1,self.y1))

class Page:
    def __init__(self, doc, index):
        self.doc=doc;self.number=index;self.page=doc.pdf[index]
        self.rect=Rect(0,0,*self.page.get_size())
        self.rotation=self.page.get_rotation()
        self.cropbox=self.page.get_bbox()
    def _point(self,x,y):
        l,b,r,t=self.cropbox;x-=l;y-=b;w=r-l;h=t-b
        return {0:(x,h-y),90:(y,x),180:(w-x,y),270:(h-y,w-x)}[self.rotation]
    def glyphs(self):
        text=self.page.get_textpage();out=[]
        try:
            for i in range(text.count_chars()):
                code=pdfium.raw.FPDFText_GetUnicode(text,i)
                if not code: continue
                char=chr(code)
                if not char.strip():continue
                l,b,r,t=text.get_charbox(i,loose=True)
                pts=[self._point(x,y) for x,y in [(l,b),(l,t),(r,b),(r,t)]]
                x=min(p[0] for p in pts);y=min(p[1] for p in pts)
                w=max(p[0] for p in pts)-x;h=max(p[1] for p in pts)-y
                if w<=0 or h<=0:continue
                ox=ctypes.c_double();oy=ctypes.c_double()
                if not pdfium.raw.FPDFText_GetCharOrigin(text,i,ox,oy):raise ValueError('missing native glyph origin')
                out.append(dict(id=f'g{len(out)}',text=char,bbox=[x,y,w,h],size=pdfium.raw.FPDFText_GetFontSize(text,i),origin=self._point(ox.value,oy.value)))
            return out
        finally:text.close()
    def words(self):
        # Use explicit native whitespace and measured character positions.
        text=self.page.get_textpage();out=[];chars=[];line=0;word=0
        def flush():
            nonlocal chars,word
            if not chars:return
            out.append((min(c[1][0] for c in chars),min(c[1][1] for c in chars),max(c[1][2] for c in chars),max(c[1][3] for c in chars),''.join(c[0] for c in chars),0,line,word));word+=1;chars=[]
        try:
            for i in range(text.count_chars()):
                code=pdfium.raw.FPDFText_GetUnicode(text,i)
                if not code:continue
                char=chr(code)
                if char.isspace():
                    flush()
                    if char=='\n':line+=1;word=0
                    continue
                l,b,r,t=text.get_charbox(i,loose=True)
                pts=[self._point(x,y) for x,y in [(l,b),(l,t),(r,b),(r,t)]]
                box=(min(p[0] for p in pts),min(p[1] for p in pts),max(p[0] for p in pts),max(p[1] for p in pts))
                chars.append((char,box))
            flush();return out
        finally:text.close()
    def render(self, dpi=72, clip=None, alpha=False):
        bitmap=self.page.render(scale=dpi/72,may_draw_forms=True,fill_color=(255,255,255,0 if alpha else 255))
        try:image=bitmap.to_pil().convert('RGBA' if alpha else 'RGB').copy()
        finally:bitmap.close()
        if clip is not None:image=image.crop(tuple(round(v*dpi/72) for v in clip))
        return image
    def get_label(self):
        return self.doc.reader.page_labels[self.number] if '/PageLabels' in self.doc.reader.trailer['/Root'] else ''

class Document:
    def __init__(self,path):
        self.name=str(path);self.reader=PdfReader(path)
        self.is_encrypted=self.reader.is_encrypted
        if self.is_encrypted:raise ValueError('目录处理暂不支持加密 PDF，请先保存不加密副本。')
        self.pdf=pdfium.PdfDocument(path);self.pdf.init_forms();self._pages={}
    def __len__(self):return len(self.pdf)
    def __getitem__(self,i):
        if i<0 or i>=len(self):raise IndexError(i)
        if i not in self._pages:self._pages[i]=Page(self,i)
        return self._pages[i]
    def __iter__(self):return (self[i] for i in range(len(self)))
    def close(self):self.pdf.close();self.reader.close()
    def __enter__(self):return self
    def __exit__(self,*args):self.close()
