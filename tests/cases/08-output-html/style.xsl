<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="html" encoding="UTF-8" doctype-public="-//W3C//DTD HTML 4.01//EN" doctype-system="http://www.w3.org/TR/html4/strict.dtd"/>
  <xsl:template match="/page">
    <html><head><title><xsl:value-of select="@title"/></title></head>
    <body><xsl:copy-of select="p"/><img src="{img/@src}" alt="ä é"/><br/><hr/>
    <xsl:copy-of select="script"/><input type="checkbox" checked="checked"/>
    <p><xsl:text disable-output-escaping="yes">&lt;b&gt;raw&lt;/b&gt;</xsl:text> &#169; &#8364;</p></body></html>
  </xsl:template>
</xsl:stylesheet>
