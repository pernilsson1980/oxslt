<?xml version="1.0"?>
<!-- exsl:document as libxslt writes it: paths relative to the file being
     written, directories made, the element's own output attributes. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:exsl="http://exslt.org/common" xmlns:e="http://exslt.org/common"
    extension-element-prefixes="exsl">
  <xsl:output method="xml" indent="no" cdata-section-elements="c"/>
  <xsl:variable name="dir" select="'made/deeper'"/>
  <xsl:template match="/">
    <main>
      <available><xsl:value-of select="element-available('exsl:document')"/></available>
      <exsl:document href="{$dir}/one.xml" indent="yes">
        <one><xsl:copy-of select="r/item"/><c>cdata from the main xsl:output</c>
          <exsl:document href="two.txt" method="text">nested: <xsl:value-of select="count(r/item)"/></exsl:document>
        </one>
      </exsl:document>
      <exsl:document href="page.html">
        <html><head><title>auto html</title></head><body><br/></body></html>
      </exsl:document>
      <exsl:document href="xhtml.xml" omit-xml-declaration="yes"
          doctype-public="-//W3C//DTD XHTML 1.0 Strict//EN"
          doctype-system="http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">
        <html xmlns="http://www.w3.org/1999/xhtml"><head/><body><p/></body></html>
      </exsl:document>
      <exsl:document href="latin1.xml" encoding="ISO-8859-1" standalone="yes"><l>café</l></exsl:document>
      <exsl:document href="empty.xml"/>
      <e:document href="literal.xml">not an extension element here</e:document>
      <after/>
    </main>
  </xsl:template>
</xsl:stylesheet>
