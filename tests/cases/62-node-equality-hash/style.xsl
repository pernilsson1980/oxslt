<?xml version="1.0"?>
<!-- libxml2 compares a node with a string (and two nodes) by xmlXPathNodeValHash first: the first two bytes of the value, but for a document node those of its first element child (or first child). So the fragment <a>1</a><b>2</b> is not equal to "12". An HTML document node (case 63, the same stylesheet with the html option) hashes as 0, so there / = "ab" is false. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:exsl="http://exslt.org/common">
<xsl:output method="text"/>
<xsl:variable name="t">12</xsl:variable>
<xsl:variable name="c"><xsl:comment>c</xsl:comment>12</xsl:variable>
<xsl:variable name="e"><a>1</a><b>2</b></xsl:variable>
<xsl:variable name="e2"><a>12</a></xsl:variable>
<xsl:variable name="te">1<a>2</a></xsl:variable>
<xsl:variable name="empty"/>
<xsl:variable name="emptyrtf"><xsl:if test="false()">x</xsl:if></xsl:variable>
<xsl:template match="/">
<xsl:value-of select="concat($t='12', $c='12', $e='12', $e2='12', $te='12', $te='2')"/>|
<xsl:value-of select="concat($e!='12', $e=$e, $e=$e2, exsl:node-set($e)='12', $emptyrtf='', $emptyrtf!='')"/>|
<xsl:value-of select="concat(/='ab', /=string(/), /!=string(/), /=/*, $e = 12, $e &lt; 13)"/>|
<xsl:value-of select="concat(exsl:node-set($te)/node()='1', count(exsl:node-set($e)|/), boolean($emptyrtf), string($e))"/>
</xsl:template>
</xsl:stylesheet>
