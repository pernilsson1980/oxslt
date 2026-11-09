<?xml version="1.0"?>
<!-- libxml2's XHTML serializer, chosen by an XHTML 1.0 DOCTYPE. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" indent="no" encoding="UTF-8"
      doctype-public="-//W3C//DTD XHTML 1.0 Transitional//EN"
      doctype-system="http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd"/>
  <xsl:template match="/">
    <html lang="en">
      <head><title>T</title></head>
      <body>
        <!-- a comment --><xsl:processing-instruction name="pi">data</xsl:processing-instruction>
        <p/><br/><img src="i.png"/><hr></hr><br>not empty</br>
        <a name="anchor">a</a><div name="d" id="given">d</div><span name="s">s</span>
        <input type="checkbox" checked="" disabled="disabled"/>
        <p xml:lang="sv">hej</p>
        <x:y xmlns:x="urn:x"/><br xmlns="urn:other"/>
        <table><tr><td></td></tr></table>
      </body>
    </html>
  </xsl:template>
</xsl:stylesheet>
