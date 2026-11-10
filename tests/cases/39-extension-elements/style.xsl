<?xml version="1.0"?>
<!-- element-available() answers for every extension element xsltproc
     registers; saxon:output and xt:document write as exsl:document does. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:exsl="http://exslt.org/common" xmlns:saxon="http://icl.com/saxon"
    xmlns:xt="http://www.jclark.com/xt" xmlns:redirect="org.apache.xalan.xslt.extensions.Redirect"
    xmlns:libxslt="http://xmlsoft.org/XSLT/namespace" xmlns:func="http://exslt.org/functions"
    xmlns:other="urn:other" extension-element-prefixes="saxon xt">
  <xsl:output method="text"/>
  <xsl:template match="/">
    <xsl:for-each select="document('')//xsl:variable[@name='names']/n">
      <xsl:value-of select="."/>=<xsl:value-of select="element-available(string(.))"/><xsl:text>&#10;</xsl:text>
    </xsl:for-each>
    <saxon:output file="saxon.xml"><by>saxon:output</by></saxon:output>
    <xt:document href="xt.xml"><by>xt:document</by></xt:document>
    <xsl:text>done&#10;</xsl:text>
  </xsl:template>
  <xsl:variable name="names">
    <n>exsl:document</n><n>saxon:output</n><n>xt:document</n><n>redirect:write</n>
    <n>libxslt:debug</n><n>func:result</n><n>func:function</n><n>other:document</n><n>xsl:document</n>
  </xsl:variable>
</xsl:stylesheet>
