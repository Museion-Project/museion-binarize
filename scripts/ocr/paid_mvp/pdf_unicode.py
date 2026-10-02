"""One CID per BMP Unicode codepoint, avoiding font glyph-alias ToUnicode losses."""
import re

def install(doc,page,font,name,characters):
 xref=page.insert_font(fontname=name,fontbuffer=font.buffer)
 descendants=doc.xref_get_key(xref,'DescendantFonts')[1];cid=int(re.search(r'(\d+) 0 R',descendants).group(1))
 glyphs=bytearray(65536*2);widths=[]
 for c in sorted(set(characters)):
  value=ord(c)
  if value>65535:raise ValueError('non-BMP text needs shared-core font extension')
  gid=font.has_glyph(value)
  if not gid and not c.isspace():raise ValueError('font missing Unicode glyph')
  glyphs[value*2:value*2+2]=int(gid).to_bytes(2,'big');widths.extend([str(value),'['+str(round(font.text_length(c,fontsize=1000),4))+']'])
 mapping=doc.get_new_xref();doc.update_object(mapping,'<<>>');doc.update_stream(mapping,bytes(glyphs));doc.xref_set_key(cid,'CIDToGIDMap',f'{mapping} 0 R');doc.xref_set_key(cid,'W','['+' '.join(widths)+']')
 cmap=doc.get_new_xref();doc.update_object(cmap,'<<>>');doc.update_stream(cmap,b'/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /PaidUnicodeIdentity def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfrange\n<0000> <FFFF> <0000>\nendbfrange\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend');doc.xref_set_key(xref,'ToUnicode',f'{cmap} 0 R');return xref

def append(doc,page,name,size,x,y,text):
 # PyMuPDF's high-level insert_text encodes glyph IDs; ours encode Unicode CIDs.
 data=f'q BT 3 Tr /{name} {size:.8f} Tf 1 0 0 1 {x:.8f} {page.rect.height-y:.8f} Tm <{text.encode("utf-16-be").hex()}> Tj ET Q\n'.encode()
 xref=doc.get_new_xref();doc.update_object(xref,'<<>>');doc.update_stream(xref,data);contents=page.get_contents()+[xref];doc.xref_set_key(page.xref,'Contents','['+' '.join(f'{x} 0 R' for x in contents)+']')
