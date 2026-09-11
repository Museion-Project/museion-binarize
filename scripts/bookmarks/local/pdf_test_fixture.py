"""Synthetic PDFs for writer tests; no external PDF toolkit dependency."""
from pypdf import PdfWriter
from pypdf.generic import NameObject, DictionaryObject, DecodedStreamObject

def make_pdf(path, texts, old_outline=False):
    writer=PdfWriter()
    font=writer._add_object(DictionaryObject({NameObject('/Type'):NameObject('/Font'),NameObject('/Subtype'):NameObject('/Type1'),NameObject('/BaseFont'):NameObject('/Helvetica')}))
    for text in texts:
        page=writer.add_blank_page(595,842)
        page[NameObject('/Resources')]=DictionaryObject({NameObject('/Font'):DictionaryObject({NameObject('/F1'):font})})
        stream=DecodedStreamObject();stream.set_data(('BT /F1 12 Tf 72 770 Td ('+text.replace('\\','\\\\').replace('(','\\(').replace(')','\\)')+') Tj ET').encode('ascii'))
        page[NameObject('/Contents')]=writer._add_object(stream)
    if old_outline:writer.add_outline_item('Old',1)
    writer.write(path);writer.close()
