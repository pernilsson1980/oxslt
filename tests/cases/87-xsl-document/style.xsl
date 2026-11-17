<!-- xsl:document, from the abandoned XSLT 1.1 draft, writes a file as
     exsl:document does (libxslt compiles both with xsltDocumentComp);
     Erlang's documentation stylesheets use it. -->
<xsl:stylesheet version="1.1" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/">
  <main available="{element-available('xsl:document')}">
    <xsl:document href="chunk.html" method="html" indent="no"><html><body><p><xsl:value-of select="doc/item"/></p></body></html></xsl:document>
  </main>
</xsl:template>
</xsl:stylesheet>
